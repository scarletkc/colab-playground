//! What `colab-playground ctl` is told, answered from the app's state as the window shows it.
//! The transport is tested in `control::owner`; only the update test here takes a lock and
//! listens, in a folder of its own.

use std::time::Duration;

use colab_playground::{
    control::{self, Claim, Code, Command, Owner, Response},
    i18n::keys,
    route::Route,
    state::POLL_CHECK,
    ui::{
        overlays::UPDATE_CONFIRM,
        pages::{UPDATE_NOTICE_INSTALL, settings::CONTROL_SELECTOR},
    },
};
use colab_playground_core::{
    backend::BackendKind as Flavor, guard::USAGE_STALE_AFTER, mock::MockFailures,
    mock::MockScenario,
};
use gpui::TestAppContext;
use serde_json::Value;
use tempfile::TempDir;

use crate::support::{ConfigDir, Launch, Options, TestApp};

const ALLOWED: &str = r#"{ "allow_cli_control": true }"#;

fn allowed(scenario: MockScenario) -> Options {
    Options {
        config: ConfigDir::TempWith(ALLOWED.into()),
        ..Options::scenario(scenario)
    }
}

fn ask(app: &mut TestApp<'_>, command: Command) -> Response {
    let state = app.state.clone();
    app.cx.update(|_, cx| control::answer(command, &state, cx))
}

fn data(response: Response) -> Value {
    assert!(response.ok, "{response:?}");
    assert_eq!(response.backend, Some(Flavor::Mock));
    response.data.expect("data")
}

#[gpui::test]
fn control_is_off_until_the_user_turns_it_on(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    for command in Command::READ {
        let refused = ask(&mut app, command);
        assert!(!refused.ok);
        assert_eq!(refused.code(), Some(Code::ControlOff), "{command:?}");
        assert_eq!(refused.backend, Some(Flavor::Mock));
        assert!(refused.data.is_none(), "nothing is read while it is off");
    }
    let message = ask(&mut app, Command::Status)
        .error
        .and_then(|failure| failure.message)
        .expect("says where to turn it on");
    for key in [
        keys::SETTINGS_CONTROL_SECTION_TITLE,
        keys::SETTINGS_CONTROL_TITLE,
    ] {
        let label = app.text(key);
        assert!(message.contains(label.as_ref()), "{message} names {label}");
    }
    assert!(
        ask(&mut app, Command::Activate).ok,
        "a second launch brings the window forward either way"
    );

    app.click(Route::Settings.nav_selector());
    app.click(CONTROL_SELECTOR);
    assert!(app.read(|state, _| state.allows_cli_control()));
    assert_eq!(
        app.saved_settings()
            .map(|settings| settings.allow_cli_control),
        Some(true)
    );
    let status = data(ask(&mut app, Command::Status));
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));

    app.click(CONTROL_SELECTOR);
    assert_eq!(
        ask(&mut app, Command::Sessions).code(),
        Some(Code::ControlOff),
        "turning it off takes effect at once"
    );
}

#[gpui::test]
fn the_answers_carry_what_the_window_shows(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, allowed(MockScenario::demo()));

    let status = data(ask(&mut app, Command::Status));
    assert_eq!(status["environment"]["state"], "ready");
    assert_eq!(status["environment"]["cli_installed"], true);
    assert_eq!(status["accounts"]["state"], "ready");
    assert_eq!(status["accounts"]["saved"], 2);
    assert_eq!(status["accounts"]["active"]["email"], "ada@example.com");
    assert_eq!(status["runtimes"]["running"], 4);
    assert_eq!(status["usage"]["known_accounts"], 2);
    assert!(status["now"].is_u64());
    assert_eq!(status["activity"], serde_json::json!([]));

    let accounts = data(ask(&mut app, Command::Accounts));
    assert_eq!(accounts["state"], "ready");
    let emails: Vec<&str> = accounts["accounts"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|account| account["email"].as_str())
        .collect();
    assert_eq!(emails, ["ada@example.com", "lin@example.com"]);
    assert!(accounts["active"].is_string());

    let sessions = data(ask(&mut app, Command::Sessions));
    assert_eq!(sessions["state"], "ready");
    let runtimes = sessions["runtimes"].as_array().expect("a list");
    assert_eq!(runtimes.len(), 4);
    let a100 = runtimes
        .iter()
        .find(|runtime| runtime["label"] == "a100-7f3a")
        .expect("the A100 session");
    assert_eq!(a100["accelerator"], "A100");
    assert_eq!(a100["managed"], true);
    assert_eq!(a100["id"]["name"], "a100-7f3a");
    for field in [
        "hardware",
        "region",
        "estimated_rate_cu_per_hour",
        "estimated_spend_cu",
        "auto_stop_at",
        "auto_stop_on_runtime_at",
        "tunnels",
        "warnings",
        "stopping",
    ] {
        assert!(a100.get(field).is_some(), "{field} is in {a100}");
    }
    let unmanaged = runtimes
        .iter()
        .find(|runtime| runtime["managed"] == false)
        .expect("the runtime opened in the browser");
    // Present and null, not left out: indexing alone reads a missing member as null too.
    assert_eq!(unmanaged["id"].get("name"), Some(&Value::Null));
    assert_eq!(sessions["unlisted_accounts"], serde_json::json!([]));

    let usage = data(ask(&mut app, Command::Usage));
    assert_eq!(usage["state"], "ready");
    assert_eq!(usage["total"]["known_accounts"], 2);
    assert_eq!(usage["total"]["unknown_accounts"], 0);
    assert!(usage["total"]["balance_cu"].is_f64());
    let accounts = usage["accounts"].as_array().expect("a list");
    assert_eq!(accounts.len(), 2);
    assert!(
        accounts
            .iter()
            .all(|account| account["snapshot"]["balance_cu"].is_f64())
    );
    assert_eq!(accounts[0]["stale"], false);
}

#[gpui::test]
fn unknown_is_never_reported_as_empty_or_zero(cx: &mut TestAppContext) {
    // Just launched: nothing has loaded yet.
    let launch = Launch::new(cx, allowed(MockScenario::demo()));
    let answer = |cx: &mut TestAppContext, command| {
        let state = launch.state.clone();
        cx.update(|cx| control::answer(command, &state, cx))
    };
    let sessions = data(answer(cx, Command::Sessions));
    assert!(
        matches!(sessions["state"].as_str(), Some("not_loaded" | "loading")),
        "{sessions}"
    );
    assert!(sessions.get("runtimes").is_none(), "not an empty list");
    let usage = data(answer(cx, Command::Usage));
    assert!(
        usage.get("total").is_none(),
        "no total over unknown accounts"
    );
    let status = data(answer(cx, Command::Status));
    assert!(status["usage"].get("balance_cu").is_none());
    assert!(status["runtimes"].get("running").is_none());
}

#[gpui::test]
fn a_failed_listing_says_so_with_the_error(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            runtimes_listing_fails: true,
            usage_fetch_fails: true,
            ..MockFailures::default()
        },
        ..allowed(MockScenario::demo())
    };
    let mut app = TestApp::with(cx, options);
    let sessions = data(ask(&mut app, Command::Sessions));
    assert_eq!(sessions["state"], "failed", "{sessions}");
    assert_eq!(sessions["error"]["kind"], "http", "{sessions}");
    assert!(sessions.get("runtimes").is_none());
    let status = data(ask(&mut app, Command::Status));
    assert_eq!(status["runtimes"]["state"], "failed");
    assert!(status["runtimes"].get("running").is_none());

    let usage = data(ask(&mut app, Command::Usage));
    assert_eq!(usage["state"], "ready", "the accounts are known");
    assert_eq!(usage["total"]["known_accounts"], 0);
    assert_eq!(
        usage["total"].get("balance_cu"),
        Some(&Value::Null),
        "{usage}"
    );
    for account in usage["accounts"].as_array().expect("a list") {
        assert_eq!(account.get("snapshot"), Some(&Value::Null), "{account}");
        assert_eq!(account.get("stale"), Some(&Value::Null), "{account}");
    }
}

#[gpui::test]
fn an_account_that_could_not_be_listed_is_named_not_left_out(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            unlisted_accounts: vec!["lin@example.com".into()],
            ..MockFailures::default()
        },
        ..allowed(MockScenario::demo())
    };
    let mut app = TestApp::with(cx, options);
    let sessions = data(ask(&mut app, Command::Sessions));
    assert_eq!(sessions["state"], "ready");
    let unlisted = sessions["unlisted_accounts"].as_array().expect("a list");
    assert_eq!(unlisted.len(), 1);
    assert_eq!(unlisted[0]["email"], "lin@example.com");
    assert!(unlisted[0]["error"]["kind"].is_string());
    let status = data(ask(&mut app, Command::Status));
    assert_eq!(status["runtimes"]["running"], 3);
    assert_eq!(status["runtimes"]["unlisted_accounts"], 1);
}

#[gpui::test]
fn values_the_window_marks_stale_are_marked_stale(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, allowed(MockScenario::demo()));
    let usage = data(ask(&mut app, Command::Usage));
    assert_eq!(usage["total"]["stale"], false);

    app.set_failures(MockFailures {
        usage_fetch_fails: true,
        ..MockFailures::default()
    });
    app.advance(USAGE_STALE_AFTER + POLL_CHECK);
    let usage = data(ask(&mut app, Command::Usage));
    assert_eq!(usage["total"]["stale"], true, "{usage}");
    for account in usage["accounts"].as_array().expect("a list") {
        assert_eq!(account["stale"], true, "{account}");
        assert!(
            account["snapshot"]["balance_cu"].is_f64(),
            "the last known values stay"
        );
    }
    let status = data(ask(&mut app, Command::Status));
    assert_eq!(status["usage"]["stale"], true);
}

/// Owns a fresh folder and serves requests from `app`, as `main` and `app::run` do.
fn serving(app: &mut TestApp<'_>) -> (TempDir, Owner) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let Claim::Owner(mut owner) =
        control::claim_within(dir.path(), Flavor::Mock, Duration::from_millis(300))
    else {
        panic!("a fresh folder has no owner");
    };
    let service = owner.listen().expect("listens");
    let state = app.state.clone();
    app.cx.update(|_, cx| control::serve(service, state, cx));
    (dir, owner)
}

fn about_to_update(cx: &mut TestAppContext, relaunch_fails: bool) -> TestApp<'_> {
    let options = Options {
        failures: MockFailures {
            update_available: true,
            update_relaunch: true,
            update_relaunch_fails: relaunch_fails,
            ..MockFailures::default()
        },
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    app.advance(POLL_CHECK);
    app.click(UPDATE_NOTICE_INSTALL);
    app
}

#[gpui::test]
fn a_restart_into_an_update_tells_a_second_launch_to_wait(cx: &mut TestAppContext) {
    let mut app = about_to_update(cx, false);
    let _owner = serving(&mut app);
    assert!(ask(&mut app, Command::Activate).ok);
    app.click(UPDATE_CONFIRM);
    assert_eq!(app.mock.update_relaunches(), 1);
    assert_eq!(
        ask(&mut app, Command::Activate).code(),
        Some(Code::Quitting),
        "the new version waits for the lock instead of bringing this window forward"
    );
}

#[gpui::test]
fn a_restart_that_failed_shows_the_window_again(cx: &mut TestAppContext) {
    let mut app = about_to_update(cx, true);
    let _owner = serving(&mut app);
    app.click(UPDATE_CONFIRM);
    assert_eq!(app.mock.update_relaunches(), 1);
    assert!(
        ask(&mut app, Command::Activate).ok,
        "this copy stays, so a second launch brings it forward"
    );
}
