//! The update check and installing an update, over the headless mock: it answers from its
//! switches, never makes an HTTP request and never replaces a file (`MockBackend` counts what
//! it was asked).

use std::time::Duration;

use colab_playground::{
    i18n::keys,
    route::Route,
    state::{Overlay, POLL_CHECK, UpdateCheck, UpdatePhase},
    ui::{
        overlays::{UPDATE_CONFIRM, UPDATE_OPEN_PAGE, UPDATE_QUIT, UPDATE_RETRY, progress_line},
        pages::{
            UPDATE_NOTICE_INSTALL, UPDATE_NOTICE_NOTES, UPDATE_NOTICE_OPEN, UPDATE_NOTICE_SELECTOR,
            settings::{UPDATE_CHECK_SELECTOR, UPDATE_INSTALL_SELECTOR, UPDATE_OPEN_SELECTOR},
        },
        sidebar::UPDATE_MARK_SELECTOR,
    },
};
use colab_playground_core::{
    mock::{MockFailures, mock_app_folder},
    update::{self, Version},
    updater::{SelfUpdate, Unsupported, UpdateProgress},
};
use gpui::TestAppContext;

use crate::support::{ConfigDir, Options, TestApp};

/// The version the mock announces: the next patch of this build.
fn next_version() -> String {
    let current = Version::parse(update::CURRENT_VERSION).expect("a semantic version");
    format!("{}.{}.{}", current.major, current.minor, current.patch + 1)
}

/// Moves the backend clock on by `by`, then lets the poller wake once (stepping the UI clock
/// through a whole day of five-second wakes would only repeat the same check).
fn later(app: &mut TestApp<'_>, by: Duration) {
    app.clock.advance(by);
    app.advance(POLL_CHECK);
}

fn available() -> MockFailures {
    MockFailures {
        update_available: true,
        ..MockFailures::default()
    }
}

#[gpui::test]
fn a_found_release_is_announced_once_and_stays_marked(cx: &mut TestAppContext) {
    let options = Options {
        failures: available(),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert_eq!(
        app.mock.update_checks(),
        0,
        "nothing before the first poller wake"
    );
    app.advance(POLL_CHECK);
    assert_eq!(
        app.mock.update_checks(),
        1,
        "checked shortly after start-up"
    );
    let version = next_version();
    assert!(app.shown(UPDATE_NOTICE_SELECTOR), "announced");
    assert!(app.shown(UPDATE_MARK_SELECTOR), "marked in the sidebar");
    let saved = app.saved_settings().expect("saved");
    assert_eq!(saved.update_latest.as_deref(), Some(version.as_str()));

    assert!(
        app.shown(UPDATE_NOTICE_INSTALL),
        "this copy can update itself"
    );
    assert!(!app.shown(UPDATE_NOTICE_OPEN), "so no download by hand");
    app.click(UPDATE_NOTICE_NOTES);
    assert_eq!(
        app.opened_url(),
        update::this_app_release_page(&version),
        "what's new: the release page, nothing downloaded"
    );
    assert_eq!(app.mock.update_installs(), 0);
    app.click("update-notice-dismiss");
    assert!(
        !app.shown(UPDATE_NOTICE_SELECTOR),
        "dismissed for this version"
    );
    assert!(
        app.shown(UPDATE_MARK_SELECTOR),
        "still marked until updated"
    );
    assert_eq!(
        app.saved_settings().and_then(|s| s.update_dismissed),
        Some(version)
    );
}

#[gpui::test]
fn after_a_restart_the_dismissed_release_is_not_announced_again(cx: &mut TestAppContext) {
    let version = next_version();
    // What the earlier run saved: release found, then dismissed.
    let saved = format!(r#"{{ "update_latest": "{version}", "update_dismissed": "{version}" }}"#);
    let options = Options {
        failures: available(),
        config: ConfigDir::TempWith(saved),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert!(
        app.shown(UPDATE_MARK_SELECTOR),
        "the release is still marked before any check"
    );
    app.advance(POLL_CHECK);
    assert_eq!(app.mock.update_checks(), 1, "every start checks");
    assert!(
        app.shown(UPDATE_MARK_SELECTOR),
        "the release is still marked"
    );
    assert!(
        !app.shown(UPDATE_NOTICE_SELECTOR),
        "but not announced again"
    );
}

#[gpui::test]
fn with_automatic_checks_off_nothing_is_asked(cx: &mut TestAppContext) {
    let options = Options {
        failures: available(),
        config: ConfigDir::TempWith(r#"{ "check_updates": false }"#.into()),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    app.advance(POLL_CHECK);
    later(&mut app, Duration::from_secs(25 * 3600));
    assert_eq!(app.mock.update_checks(), 0);
    assert!(!app.shown(UPDATE_NOTICE_SELECTOR));
}

#[gpui::test]
fn a_manual_check_shows_each_state(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith(r#"{ "check_updates": false }"#.into()),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    app.click(Route::Settings.nav_selector());
    assert!(app.shown("update-status idle"));

    app.update(|state, cx| {
        state.check_updates_now(cx);
        assert_eq!(state.update_check(), &UpdateCheck::Checking);
    });
    assert!(app.shown("update-status up-to-date"));

    app.set_failures(MockFailures {
        update_check_fails: true,
        ..MockFailures::default()
    });
    app.click(UPDATE_CHECK_SELECTOR);
    assert!(app.shown("update-status failed"));
    let failed = app.text(keys::UPDATE_FAILED_TITLE);
    assert!(
        app.toasts().contains(&failed),
        "a manual failure is reported"
    );

    app.set_failures(available());
    app.click(UPDATE_CHECK_SELECTOR);
    assert!(app.shown("update-status available"));
    assert!(app.shown(UPDATE_INSTALL_SELECTOR));
    assert!(!app.shown(UPDATE_OPEN_SELECTOR));
    assert_eq!(app.mock.update_checks(), 3, "every check went to the mock");
}

#[gpui::test]
fn an_automatic_failure_stays_quiet(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            update_check_fails: true,
            ..MockFailures::default()
        },
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    app.advance(POLL_CHECK);
    assert_eq!(app.mock.update_checks(), 1);
    let failed = app.text(keys::UPDATE_FAILED_TITLE);
    assert!(
        !app.toasts().contains(&failed),
        "no toast for an automatic check"
    );
    app.click(Route::Settings.nav_selector());
    assert!(app.shown("update-status failed"), "shown in About");
    later(&mut app, Duration::from_secs(600));
    assert_eq!(app.mock.update_checks(), 1, "throttled within the hour");

    app.set_failures(available());
    later(&mut app, Duration::from_secs(3600));
    assert_eq!(app.mock.update_checks(), 2, "retried after an hour");
    assert!(app.shown(UPDATE_MARK_SELECTOR));
    later(&mut app, Duration::from_secs(3600));
    assert_eq!(app.mock.update_checks(), 2, "answered, so not asked again");
}

#[gpui::test]
fn it_checks_once_per_start(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.advance(POLL_CHECK);
    assert_eq!(app.mock.update_checks(), 1);
    later(&mut app, Duration::from_secs(3600));
    later(&mut app, Duration::from_secs(25 * 3600));
    assert_eq!(app.mock.update_checks(), 1, "not again while the app runs");
}

fn with(extra: MockFailures) -> Options {
    Options {
        failures: MockFailures {
            update_available: true,
            ..extra
        },
        ..Options::default()
    }
}

/// The app with a newer release announced, the confirmation open from the banner.
fn confirming(cx: &mut TestAppContext, failures: MockFailures) -> TestApp<'_> {
    let mut app = TestApp::with(cx, with(failures));
    app.advance(POLL_CHECK);
    app.click(UPDATE_NOTICE_INSTALL);
    assert!(app.shown("update-phase confirm"));
    app
}

fn phase(app: &mut TestApp<'_>) -> Option<UpdatePhase> {
    app.read(|state, _| state.update_prompt().map(|prompt| prompt.phase.clone()))
}

#[gpui::test]
fn download_and_restart_says_what_a_restart_affects_before_anything_starts(
    cx: &mut TestAppContext,
) {
    let mut app = confirming(cx, MockFailures::default());
    for consequence in ["forwards", "billing", "auto-stop"] {
        assert!(
            app.shown(&format!("update-consequence {consequence}")),
            "{consequence}"
        );
    }
    assert!(!app.shown("update-blocked"));
    assert_eq!(
        app.mock.update_installs(),
        0,
        "nothing before the confirmation"
    );
    app.click("overlay-cancel");
    assert_eq!(phase(&mut app), None, "Cancel closes it");
    assert_eq!(app.mock.update_installs(), 0);
    assert!(app.shown(UPDATE_NOTICE_SELECTOR), "the banner stays");
}

#[gpui::test]
fn each_step_shows_what_happens_now_and_cancel_until_installing(cx: &mut TestAppContext) {
    let mut app = confirming(cx, MockFailures::default());
    app.update_and_draw(|state, cx| {
        state.confirm_update(cx);
        assert!(state.update_cancellable());
    });
    assert!(app.shown("update-phase preparing"));
    let mib = 1024 * 1024;
    app.update_and_draw(|state, cx| {
        state.apply_update_progress(
            UpdateProgress::Downloading {
                received: 15 * mib,
                total: Some(30 * mib),
            },
            cx,
        )
    });
    assert!(app.shown("update-phase downloading"));
    assert!(app.shown("update-steps"), "the steps still to come");
    let line = app.read(|state, cx| {
        let phase = &state.update_prompt().unwrap().phase;
        progress_line(phase, false, cx)
    });
    assert_eq!(line.text.as_ref(), "Downloading… 15.0 MB of 30.0 MB");
    assert_eq!(line.share, Some(0.5));
    assert_eq!(line.step, 0);
    app.update_and_draw(|state, cx| state.apply_update_progress(UpdateProgress::Verifying, cx));
    assert!(app.shown("update-phase verifying"));
    assert!(app.read(|state, _| state.update_cancellable()));
    app.update_and_draw(|state, cx| state.apply_update_progress(UpdateProgress::Installing, cx));
    assert!(app.shown("update-phase installing"));
    assert!(
        !app.read(|state, _| state.update_cancellable()),
        "no cancel once the new version is being put in place"
    );
    app.update_and_draw(|state, cx| {
        state.close_overlay(cx);
        state.request_quit(cx);
        assert!(
            state.update_prompt().is_some() && !state.quit_approved(),
            "neither closing the dialog nor quitting stops it halfway"
        );
    });
    app.settle();
    assert_eq!(
        app.mock.update_installs(),
        1,
        "the simulated update finished"
    );
}

#[gpui::test]
fn the_demo_simulates_the_update_and_keeps_running(cx: &mut TestAppContext) {
    let mut app = confirming(cx, MockFailures::default());
    app.click(UPDATE_CONFIRM);
    assert_eq!(app.mock.update_installs(), 1);
    assert_eq!(phase(&mut app), None);
    let simulated = app.text(keys::UPDATE_SIMULATED_TITLE);
    assert!(app.toasts().contains(&simulated), "{:?}", app.toasts());
    assert!(
        !app.read(|state, _| state.quit_approved()),
        "nothing restarts"
    );
    assert_eq!(app.mock.update_relaunches(), 0);
}

#[gpui::test]
fn settings_about_offers_the_same_action(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, with(MockFailures::default()));
    app.advance(POLL_CHECK);
    app.click(Route::Settings.nav_selector());
    assert!(app.shown("update-status available"));
    app.click(UPDATE_INSTALL_SELECTOR);
    assert!(app.shown("update-phase confirm"));
}

#[gpui::test]
fn a_cancelled_download_changes_nothing(cx: &mut TestAppContext) {
    let mut app = confirming(cx, MockFailures::default());
    // Preparing, then three download steps in.
    app.cancel_switch.arm(4);
    app.click(UPDATE_CONFIRM);
    assert_eq!(phase(&mut app), None, "the dialog closes");
    assert_eq!(app.mock.update_installs(), 0);
    assert_eq!(app.saved_settings().and_then(|s| s.update_unverified), None);
    assert!(
        app.shown(UPDATE_NOTICE_INSTALL),
        "and it can be tried again"
    );
}

#[gpui::test]
fn the_dialog_opens_again_only_once_a_cancelled_download_has_returned(cx: &mut TestAppContext) {
    // The demo scenario has runtimes billing, so a quit asks first.
    let mut app = confirming(cx, MockFailures::default());
    app.update_and_draw(|state, cx| {
        state.confirm_update(cx);
        state.request_quit(cx);
        assert!(matches!(state.overlay(), Some(Overlay::Quit(_))));
        state.close_overlay(cx);
        state.begin_update(cx);
        assert!(
            state.update_prompt().is_none(),
            "not while the cancelled download is still returning"
        );
    });
    app.settle();
    assert_eq!(app.mock.update_installs(), 0);
    app.click(UPDATE_NOTICE_INSTALL);
    app.settle();
    assert!(app.shown("update-phase confirm"), "and it stays open");
}

#[gpui::test]
fn a_cancel_once_the_new_version_is_being_put_in_place_does_not_stop_it(cx: &mut TestAppContext) {
    // The simulation waits once to prepare, sixteen times to download, once to check and once
    // to put the new version in place.
    let mut app = confirming(cx, MockFailures::default());
    app.cancel_switch.arm(18);
    app.click(UPDATE_CONFIRM);
    assert_eq!(phase(&mut app), None, "during the check a cancel applies");
    assert_eq!(app.mock.update_installs(), 0);
    app.click(UPDATE_NOTICE_INSTALL);
    app.cancel_switch.arm(19);
    app.click(UPDATE_CONFIRM);
    assert_eq!(
        app.mock.update_installs(),
        1,
        "past the point of no return the update finishes"
    );
}

#[gpui::test]
fn only_the_release_the_dialog_names_is_installed(cx: &mut TestAppContext) {
    let current = Version::parse(update::CURRENT_VERSION).expect("a semantic version");
    let saved = format!("{}.{}.{}", current.major, current.minor, current.patch + 2);
    let options = Options {
        failures: available(),
        config: ConfigDir::TempWith(format!(
            r#"{{ "check_updates": false, "update_latest": "{saved}" }}"#
        )),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    app.click(Route::Settings.nav_selector());
    app.click(UPDATE_INSTALL_SELECTOR);
    let named = |app: &mut TestApp<'_>| {
        app.read(|state, _| state.update_prompt().map(|prompt| prompt.version.clone()))
    };
    assert_eq!(named(&mut app), Some(saved));
    // A check finds another release while the dialog is open.
    app.update(|state, cx| state.check_updates_now(cx));
    app.settle();
    app.click(UPDATE_CONFIRM);
    assert_eq!(
        app.mock.update_installs(),
        0,
        "not before that one is confirmed"
    );
    assert!(app.shown("update-phase confirm"));
    assert_eq!(named(&mut app), Some(next_version()));
    app.click(UPDATE_CONFIRM);
    assert_eq!(app.mock.update_installs(), 1);
}

#[gpui::test]
fn an_unverified_download_is_not_installed_and_never_offered_by_hand(cx: &mut TestAppContext) {
    let mut app = confirming(
        cx,
        MockFailures {
            update_unverified: true,
            ..MockFailures::default()
        },
    );
    app.click(UPDATE_CONFIRM);
    assert!(app.shown("update-phase failed unverified"));
    assert!(!app.shown(UPDATE_OPEN_PAGE), "not the same files by hand");
    assert!(!app.shown(UPDATE_RETRY));
    assert_eq!(app.mock.update_installs(), 0);
    let version = next_version();
    assert_eq!(
        app.saved_settings().and_then(|s| s.update_unverified),
        Some(version.clone()),
        "remembered across restarts"
    );
    app.click("overlay-cancel");
    assert!(!app.shown(UPDATE_NOTICE_SELECTOR), "no longer announced");
    app.click(Route::Settings.nav_selector());
    assert!(app.shown("update-status unverified"));
    assert!(!app.shown(UPDATE_OPEN_SELECTOR));
    app.update(|state, cx| state.open_update_page(cx));
    assert_eq!(app.opened_url(), None, "its page is not opened either");
}

#[gpui::test]
fn after_a_restart_an_unverified_release_stays_unoffered(cx: &mut TestAppContext) {
    let version = next_version();
    let saved = format!(r#"{{ "update_latest": "{version}", "update_unverified": "{version}" }}"#);
    let options = Options {
        failures: MockFailures {
            update_unsupported: true,
            ..MockFailures::default()
        },
        config: ConfigDir::TempWith(saved),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert!(!app.shown(UPDATE_NOTICE_SELECTOR));
    app.click(Route::Settings.nav_selector());
    assert!(app.shown("update-status unverified"));
    assert!(
        !app.shown(UPDATE_OPEN_SELECTOR),
        "even a copy that cannot update itself is not sent to those files"
    );
}

#[gpui::test]
fn a_failed_download_or_install_keeps_this_version_and_offers_the_way_on(cx: &mut TestAppContext) {
    let failures = [
        MockFailures {
            update_download_fails: true,
            ..MockFailures::default()
        },
        MockFailures {
            update_install_fails: true,
            ..MockFailures::default()
        },
    ];
    let mut app = TestApp::with(cx, with(MockFailures::default()));
    app.advance(POLL_CHECK);
    for failure in failures {
        app.set_failures(MockFailures {
            update_available: true,
            ..failure
        });
        app.click(UPDATE_NOTICE_INSTALL);
        app.click(UPDATE_CONFIRM);
        assert!(app.shown("update-phase failed error"));
        assert_eq!(app.mock.update_installs(), 0);
        app.click(UPDATE_OPEN_PAGE);
        assert_eq!(
            app.opened_url(),
            update::this_app_release_page(&next_version()),
            "downloading by hand is offered"
        );
        app.click("overlay-cancel");
    }
    app.set_failures(with(MockFailures::default()).failures);
    app.click(UPDATE_NOTICE_INSTALL);
    app.set_failures(MockFailures {
        update_available: true,
        update_install_fails: true,
        ..MockFailures::default()
    });
    app.click(UPDATE_CONFIRM);
    app.set_failures(with(MockFailures::default()).failures);
    app.click(UPDATE_RETRY);
    assert_eq!(app.mock.update_installs(), 1, "Try again runs it again");
}

#[gpui::test]
fn a_copy_that_cannot_replace_itself_says_why_and_offers_the_download_page(
    cx: &mut TestAppContext,
) {
    let mut app = TestApp::with(
        cx,
        with(MockFailures {
            update_unsupported: true,
            ..MockFailures::default()
        }),
    );
    app.advance(POLL_CHECK);
    assert_eq!(
        app.read(|state, _| state.self_update().cloned()),
        Some(SelfUpdate::Unsupported(Unsupported::NotWritable {
            folder: mock_app_folder().to_owned()
        }))
    );
    assert!(app.shown(UPDATE_NOTICE_OPEN));
    assert!(!app.shown(UPDATE_NOTICE_INSTALL));
    app.update(|state, cx| state.begin_update(cx));
    assert_eq!(phase(&mut app), None, "no install to start");
    app.click(Route::Settings.nav_selector());
    assert!(app.shown("update-status available-manual"));
    assert!(app.shown(UPDATE_OPEN_SELECTOR));
    assert!(!app.shown(UPDATE_INSTALL_SELECTOR));
    app.click(UPDATE_OPEN_SELECTOR);
    assert_eq!(
        app.opened_url(),
        update::this_app_release_page(&next_version())
    );
}

#[gpui::test]
fn updating_waits_while_a_deployment_runs(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, with(MockFailures::default()));
    app.advance(POLL_CHECK);
    app.click(Route::Plugins.nav_selector());
    app.click("plugin-run llama-cpp");
    app.update_and_draw(|state, cx| {
        state.edit_run_form(|form| form.existing = Some("a100-7f3a".into()), cx);
        state.start_run(
            vec![("repo".into(), "unsloth/Qwen3-0.6B-GGUF".into())],
            String::new(),
            cx,
        );
        state.begin_update(cx);
        assert_eq!(state.update_block(), Some(keys::UPDATE_BLOCKED_RUN));
        state.confirm_update(cx);
        assert!(
            matches!(
                state.update_prompt().map(|p| &p.phase),
                Some(UpdatePhase::Confirm)
            ),
            "nothing starts while it runs"
        );
    });
    assert!(app.shown("update-blocked"));
    app.settle();
    assert_eq!(app.mock.update_installs(), 0);
}

#[gpui::test]
fn the_restart_quits_without_asking_again_once_the_update_is_in_place(cx: &mut TestAppContext) {
    // The demo scenario has runtimes billing, which would otherwise open the quit prompt.
    let mut app = confirming(
        cx,
        MockFailures {
            update_relaunch: true,
            ..MockFailures::default()
        },
    );
    app.click(UPDATE_CONFIRM);
    assert_eq!(app.mock.update_installs(), 1);
    assert_eq!(
        app.mock.update_relaunches(),
        1,
        "the new version was started"
    );
    assert!(app.read(|state, _| state.quit_approved()));
    assert!(
        !app.read(|state, _| matches!(state.overlay(), Some(Overlay::Quit(_)))),
        "the confirmation already said what it affects"
    );
}

#[gpui::test]
fn when_the_new_version_cannot_be_started_the_app_says_so_and_stays(cx: &mut TestAppContext) {
    let mut app = confirming(
        cx,
        MockFailures {
            update_relaunch: true,
            update_relaunch_fails: true,
            ..MockFailures::default()
        },
    );
    app.click(UPDATE_CONFIRM);
    assert!(app.shown("update-phase restart-failed"));
    assert!(!app.read(|state, _| state.quit_approved()));
    app.click(UPDATE_QUIT);
    assert!(
        app.read(|state, _| matches!(state.overlay(), Some(Overlay::Quit(_)))),
        "Quit goes through the usual question about what is billing"
    );
}
