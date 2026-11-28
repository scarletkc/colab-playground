//! Counts and "nothing is running" never come from what the app does not know: the Sessions
//! empty state needs a complete listing, the Overview runtime count (from usage) is unknown
//! without any reading and marked when it leaves accounts out, and the switch dialog needs to
//! know what the account being left still runs.

use colab_playground::{
    i18n::keys,
    route::Route,
    state::{AccountDialog, Load},
    ui::{
        dialogs::{
            SWITCH_NOTHING_SELECTOR, SWITCH_SESSIONS_UNWATCHED_SELECTOR,
            SWITCH_SESSIONS_WATCHED_SELECTOR, SWITCH_UNMANAGED_UNKNOWN_SELECTOR,
            switch_session_app_only_selector, switch_session_deadline_selector,
            switch_session_selector,
        },
        overlays::CANCEL_AUTO_STOP_CONFIRM,
        pages::{
            accounts::{ADD_SELECTOR, switch_selector},
            overview::{RUNTIMES_INCOMPLETE_SELECTOR, RUNTIMES_UNCOUNTED_SELECTOR},
            sessions::{
                EMPTY_PARTIAL_SELECTOR, EMPTY_SELECTOR, cancel_auto_stop_selector,
                set_auto_stop_selector,
            },
        },
    },
};
use colab_playground_core::mock::{MockAccount, MockFailures, MockScenario};
use gpui::TestAppContext;

use crate::support::{Options, TestApp};

const LISTING_FAILED: &str = "error-sessions";

fn two_accounts_nothing_running() -> MockScenario {
    MockScenario {
        accounts: vec![
            MockAccount {
                email: "ada@example.com".into(),
                balance_cu: 100.0,
            },
            MockAccount {
                email: "lin@example.com".into(),
                balance_cu: 50.0,
            },
        ],
        runtimes: vec![],
    }
}

fn on_sessions(app: &mut TestApp<'_>) {
    app.click(Route::Sessions.nav_selector());
    assert_eq!(app.route(), Route::Sessions);
}

/// Opens the switch dialog from ada (active) to lin; how many unmanaged runtimes it was
/// given, if they could be listed.
fn ask_to_switch(app: &mut TestApp<'_>) -> Option<usize> {
    app.click(Route::Accounts.nav_selector());
    app.click(&switch_selector("lin@example.com"));
    match app.read(|state, _| state.dialog().cloned()) {
        Some(AccountDialog::Switch {
            impact: Load::Ready((impact, _)),
            ..
        }) => impact.unmanaged_left_running.map(|listed| listed.len()),
        other => panic!("expected the switch dialog with its impact, got {other:?}"),
    }
}

#[gpui::test]
fn a_complete_empty_listing_says_nothing_is_running(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    on_sessions(&mut app);
    assert!(app.shown(EMPTY_SELECTOR));
    assert!(!app.shown(EMPTY_PARTIAL_SELECTOR));
}

#[gpui::test]
fn a_failed_listing_never_says_nothing_is_running(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            runtimes_listing_fails: true,
            ..MockFailures::default()
        },
        ..Options::scenario(MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    on_sessions(&mut app);
    assert!(app.shown(LISTING_FAILED), "it says the listing failed");
    assert!(!app.shown(EMPTY_SELECTOR));
    assert!(!app.shown(EMPTY_PARTIAL_SELECTOR));
}

#[gpui::test]
fn a_partial_listing_says_only_the_listed_accounts_are_empty(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            unlisted_accounts: vec!["lin@example.com".into()],
            ..MockFailures::default()
        },
        ..Options::scenario(two_accounts_nothing_running())
    };
    let mut app = TestApp::with(cx, options);
    on_sessions(&mut app);
    let lin = app.read(|state, _| {
        let accounts = &state.accounts().ready().expect("accounts").accounts;
        let lin = accounts
            .iter()
            .find(|account| account.email.as_deref() == Some("lin@example.com"));
        lin.expect("the second account").id.clone()
    });
    assert!(
        app.shown(&format!("error-sessions-account-{lin}")),
        "the account that could not be listed is named"
    );
    assert!(app.shown(EMPTY_PARTIAL_SELECTOR));
    assert!(!app.shown(EMPTY_SELECTOR), "not \"Nothing is running\"");
}

#[gpui::test]
fn a_listing_in_flight_never_says_nothing_is_running(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            runtimes_listing_fails: true,
            ..MockFailures::default()
        },
        ..Options::scenario(MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    on_sessions(&mut app);
    app.set_failures(MockFailures::default());
    // A failed list goes back to loading while it is fetched again.
    app.update_and_draw(|state, cx| state.load_runtimes(cx));
    assert!(app.read(|state, _| state.runtimes().is_loading()));
    assert!(!app.shown(EMPTY_SELECTOR));
    assert!(!app.shown(LISTING_FAILED));
    app.settle();
    assert!(app.shown(EMPTY_SELECTOR), "the listing came back empty");
}

#[gpui::test]
fn the_runtime_count_counts_every_account(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    assert_eq!(app.read(|state, _| state.usage_summary().known), 2);
    assert!(app.shown("summary runtimes 4"));
    assert!(!app.shown(RUNTIMES_INCOMPLETE_SELECTOR));
    assert!(!app.shown(RUNTIMES_UNCOUNTED_SELECTOR));
}

#[gpui::test]
fn without_any_usage_the_runtime_count_is_unknown(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            usage_fetch_fails: true,
            ..MockFailures::default()
        },
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert_eq!(app.read(|state, _| state.usage_summary().known), 0);
    let unknown = app.text(keys::SUMMARY_UNKNOWN);
    assert!(app.shown(&format!("summary runtimes {unknown}")));
    assert!(!app.shown("summary runtimes 0"), "never zero");
    assert!(!app.shown(RUNTIMES_INCOMPLETE_SELECTOR));
}

#[gpui::test]
fn an_account_without_usage_is_left_out_of_the_runtime_count_and_named(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    // A new account whose usage never loads: the others keep their last readings.
    app.set_failures(MockFailures {
        usage_fetch_fails: true,
        ..MockFailures::default()
    });
    app.click(Route::Accounts.nav_selector());
    app.click(ADD_SELECTOR);
    app.click("add-continue");
    app.fill(|inputs| inputs.code.clone(), "4/0AbCdEf");
    app.click("add-submit");
    let (uncounted, added) =
        app.read(|state, _| (state.uncounted_accounts(), state.active_email()));
    assert_eq!(
        uncounted,
        vec![added],
        "the account just added (now active)"
    );
    app.click(Route::Overview.nav_selector());
    assert!(
        app.shown("summary runtimes 4"),
        "the accounts with readings still add up"
    );
    assert!(app.shown(RUNTIMES_INCOMPLETE_SELECTOR), "marked incomplete");
    assert!(
        app.shown(RUNTIMES_UNCOUNTED_SELECTOR),
        "and says what is left out"
    );
}

#[gpui::test]
fn switching_away_from_an_idle_account_says_nothing_is_running(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(two_accounts_nothing_running()));
    assert_eq!(ask_to_switch(&mut app), Some(0));
    assert!(app.shown(SWITCH_NOTHING_SELECTOR));
    assert!(!app.shown(SWITCH_UNMANAGED_UNKNOWN_SELECTOR));
}

#[gpui::test]
fn switching_never_says_nothing_is_running_when_the_runtimes_are_unknown(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            unlisted_accounts: vec!["ada@example.com".into()],
            usage_fetch_fails: true,
            ..MockFailures::default()
        },
        ..Options::scenario(two_accounts_nothing_running())
    };
    let mut app = TestApp::with(cx, options);
    assert_eq!(ask_to_switch(&mut app), None, "neither listed nor counted");
    assert!(
        app.shown(SWITCH_UNMANAGED_UNKNOWN_SELECTOR),
        "it says they could not be checked"
    );
    assert!(
        !app.shown(SWITCH_NOTHING_SELECTOR),
        "not \"Nothing is running\""
    );

    app.click("switch-confirm");
    assert_eq!(
        app.read(|state, _| state.active_email()).as_deref(),
        Some("lin@example.com"),
        "the switch stays possible"
    );
}

#[gpui::test]
fn switching_never_lets_usage_stand_in_for_runtimes_it_could_not_list(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            unlisted_accounts: vec!["ada@example.com".into()],
            ..MockFailures::default()
        },
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    let read = app.read(|state, _| state.usage_summary().known);
    assert_eq!(read, 2, "usage was read");
    assert_eq!(ask_to_switch(&mut app), None, "the reading is not a list");
    assert!(
        app.shown(&switch_session_selector("a100-7f3a")),
        "the managed sessions are listed"
    );
    assert!(
        app.shown(SWITCH_UNMANAGED_UNKNOWN_SELECTOR),
        "and the warning comes with them"
    );
    assert!(!app.shown(SWITCH_NOTHING_SELECTOR));
}

#[gpui::test]
fn switching_never_promises_auto_stop_after_watchdog_is_cancelled(cx: &mut TestAppContext) {
    use colab_playground_core::{CancelToken, backend::Backend as _};

    let mut app = TestApp::new(cx);
    let runtimes = app.mock.runtimes(&CancelToken::new()).unwrap().runtimes;
    let a100 = runtimes
        .iter()
        .find(|r| r.id.name.as_deref() == Some("a100-7f3a"))
        .unwrap()
        .id
        .clone();
    let t4 = runtimes
        .iter()
        .find(|r| r.id.name.as_deref() == Some("t4-91c2"))
        .unwrap()
        .id
        .clone();

    // Cancel auto-stop on a100 while t4 keeps its armed watchdog.
    app.mock
        .cancel_auto_stop(&a100, &CancelToken::new())
        .unwrap();
    ask_to_switch(&mut app);
    assert!(
        app.shown(SWITCH_SESSIONS_WATCHED_SELECTOR),
        "t4 still stops automatically"
    );
    assert!(app.shown(&switch_session_deadline_selector("t4-91c2")));
    assert!(!app.shown(&switch_session_app_only_selector("t4-91c2")));
    assert!(
        app.shown(SWITCH_SESSIONS_UNWATCHED_SELECTOR),
        "a100 is grouped as having no automatic stop"
    );
    assert!(app.shown(&switch_session_selector("a100-7f3a")));
    assert!(
        !app.shown(&switch_session_deadline_selector("a100-7f3a")),
        "cancelled watchdog must not display a countdown"
    );
    assert!(!app.shown(&switch_session_app_only_selector("a100-7f3a")));

    // Cancel auto-stop on the remaining managed session too.
    app.click("dialog-cancel");
    app.mock.cancel_auto_stop(&t4, &CancelToken::new()).unwrap();
    ask_to_switch(&mut app);
    assert!(
        !app.shown(SWITCH_SESSIONS_WATCHED_SELECTOR),
        "auto-stop heading disappears when no watchdog is armed"
    );
    assert!(app.shown(SWITCH_SESSIONS_UNWATCHED_SELECTOR));
    assert!(!app.shown(&switch_session_deadline_selector("a100-7f3a")));
    assert!(!app.shown(&switch_session_deadline_selector("t4-91c2")));
}

#[gpui::test]
fn switching_groups_app_only_auto_stop_as_watched_and_notes_app_must_stay_open(
    cx: &mut TestAppContext,
) {
    let mut app = TestApp::new(cx);
    on_sessions(&mut app);
    app.click(&cancel_auto_stop_selector("a100-7f3a"));
    app.click(CANCEL_AUTO_STOP_CONFIRM);
    app.set_failures(MockFailures {
        watchdog_restart_fails: true,
        ..MockFailures::default()
    });
    app.click(&set_auto_stop_selector("a100-7f3a"));

    ask_to_switch(&mut app);
    assert!(app.shown(SWITCH_SESSIONS_WATCHED_SELECTOR));
    assert!(
        !app.shown(SWITCH_SESSIONS_UNWATCHED_SELECTOR),
        "app-side runtime limit still stops the session while the app stays open"
    );
    assert!(app.shown(&switch_session_deadline_selector("a100-7f3a")));
    assert!(
        app.shown(&switch_session_app_only_selector("a100-7f3a")),
        "app-only limit notes that it applies only while the app is open"
    );
    assert!(app.shown(&switch_session_deadline_selector("t4-91c2")));
    assert!(
        !app.shown(&switch_session_app_only_selector("t4-91c2")),
        "armed watchdog is not marked app-only"
    );

    // Cancel auto-stop on t4 so both groups appear alongside the app-only session.
    app.click("dialog-cancel");
    on_sessions(&mut app);
    app.click(&cancel_auto_stop_selector("t4-91c2"));
    app.click(CANCEL_AUTO_STOP_CONFIRM);
    ask_to_switch(&mut app);
    assert!(app.shown(SWITCH_SESSIONS_WATCHED_SELECTOR));
    assert!(app.shown(&switch_session_deadline_selector("a100-7f3a")));
    assert!(app.shown(&switch_session_app_only_selector("a100-7f3a")));
    assert!(app.shown(SWITCH_SESSIONS_UNWATCHED_SELECTOR));
    assert!(app.shown(&switch_session_selector("t4-91c2")));
    assert!(!app.shown(&switch_session_deadline_selector("t4-91c2")));
    assert!(!app.shown(&switch_session_app_only_selector("t4-91c2")));
}
