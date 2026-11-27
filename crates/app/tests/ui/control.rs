//! What `colab-playground ctl` is told, and what it does, answered from the app's state as the
//! window shows it. The transport is tested in `control::owner`; only the update test here takes
//! a lock and listens, in a folder of its own.
//!
//! A headless backend call runs to its end at the next settle, so an operation is seen running
//! by asking before settling.

use std::time::Duration;

use colab_playground::{
    control::{
        self, CancelArgs, Claim, Code, Command, OperationArgs, Owner, Response, StartArgs, StopArgs,
    },
    i18n::{self, keys},
    route::Route,
    state::{Overlay, POLL_CHECK, ProvisionPhase},
    ui::{
        overlays::UPDATE_CONFIRM,
        pages::{
            UPDATE_NOTICE_INSTALL,
            sessions::{NEW_SESSION_SELECTOR, PROVISIONING_SELECTOR},
            settings::{CONTROL_CHANGES_SELECTOR, CONTROL_SELECTOR},
        },
    },
};
use colab_playground_core::{
    CancelToken,
    backend::{Backend, BackendKind as Flavor},
    guard::USAGE_STALE_AFTER,
    mock::{MockFailures, MockScenario},
    sessions::Accelerator,
};
use gpui::TestAppContext;
use serde_json::Value;
use tempfile::TempDir;

use crate::support::{ConfigDir, Launch, Options, TestApp};

const ALLOWED: &str = r#"{ "allow_cli_control": true }"#;
const CHANGES_ALLOWED: &str = r#"{ "allow_cli_control": true, "allow_cli_changes": true }"#;
/// The demo's active account already has two named runtimes, the default limit.
const CHANGES_ALLOWED_ROOMY: &str = r#"{
    "allow_cli_control": true,
    "allow_cli_changes": true,
    "defaults": { "max_concurrent_sessions": 5 }
}"#;

fn allowed(scenario: MockScenario) -> Options {
    Options {
        config: ConfigDir::TempWith(ALLOWED.into()),
        ..Options::scenario(scenario)
    }
}

fn changes_allowed(settings: &str, scenario: MockScenario) -> Options {
    Options {
        config: ConfigDir::TempWith(settings.into()),
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

fn refusal(response: Response) -> Code {
    assert!(!response.ok, "{response:?}");
    assert_eq!(response.backend, Some(Flavor::Mock));
    assert!(
        response.data.is_none(),
        "nothing is done or told: {response:?}"
    );
    let failure = response.error.expect("why");
    assert!(
        failure
            .message
            .as_deref()
            .is_some_and(|text| !text.is_empty()),
        "{:?} says why",
        failure.code
    );
    failure.code
}

fn start(accelerator: Option<Accelerator>, name: Option<&str>, yes: bool) -> Command {
    Command::Start(StartArgs {
        accelerator,
        name: name.map(str::to_owned),
        yes,
    })
}

fn stop(runtime: &str, account: Option<&str>, yes: bool) -> Command {
    Command::Stop(StopArgs {
        runtime: runtime.into(),
        account: account.map(str::to_owned),
        yes,
    })
}

fn operation(id: &str) -> Command {
    Command::Operation(OperationArgs { id: id.into() })
}

fn cancel(id: &str, yes: bool) -> Command {
    Command::Cancel(CancelArgs { id: id.into(), yes })
}

/// The operation in an answer, and its id.
fn followed(response: Response) -> (Value, String) {
    let operation = data(response)["operation"].clone();
    let id = operation["id"].as_str().expect("an id").to_owned();
    (operation, id)
}

fn listed(app: &mut TestApp<'_>) -> Vec<Value> {
    data(ask(app, Command::Sessions))["runtimes"]
        .as_array()
        .expect("listed")
        .clone()
}

fn listed_names(app: &mut TestApp<'_>) -> Vec<String> {
    listed(app)
        .iter()
        .map(|runtime| runtime["label"].as_str().expect("a label").to_owned())
        .collect()
}

fn on_sessions(app: &mut TestApp<'_>) {
    app.click(Route::Sessions.nav_selector());
    assert_eq!(app.route(), Route::Sessions);
}

fn toast_text(app: &mut TestApp<'_>, key: &str, name: &str) -> gpui::SharedString {
    app.cx
        .update(|_, cx| i18n::tr_args(cx, key, &[("name", name)]))
}

#[gpui::test]
fn control_is_off_until_the_user_turns_it_on(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    for command in Command::READ {
        let refused = ask(&mut app, command.clone());
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

#[gpui::test]
fn a_copy_that_only_allowed_reading_changes_nothing(cx: &mut TestAppContext) {
    // As a settings file from before starting and stopping could be allowed.
    let mut app = TestApp::with(cx, allowed(MockScenario::demo()));
    for command in [
        start(None, None, false),
        start(Some(Accelerator::T4), None, true),
        stop("a100-7f3a", None, false),
        stop("a100-7f3a", None, true),
        cancel("anything", true),
    ] {
        let response = ask(&mut app, command.clone());
        let message = response
            .error
            .as_ref()
            .and_then(|failure| failure.message.clone())
            .unwrap_or_default();
        assert_eq!(refusal(response), Code::ChangesOff, "{command:?}");
        let setting = app.text(keys::SETTINGS_CONTROL_CHANGES_TITLE);
        assert!(
            message.contains(setting.as_ref()),
            "{message} names {setting}"
        );
    }
    assert!(!app.read(|state, _| state.is_provisioning()));
    app.settle();
    assert_eq!(listed(&mut app).len(), 4, "nothing was stopped");
    assert!(ask(&mut app, Command::Sessions).ok, "reading still works");
    assert_eq!(
        refusal(ask(&mut app, operation("anything"))),
        Code::UnknownOperation,
        "following an operation only reads"
    );
}

#[gpui::test]
fn starting_and_stopping_are_a_second_step_in_settings(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    app.click(Route::Settings.nav_selector());
    app.click(CONTROL_CHANGES_SELECTOR);
    assert!(
        !app.read(|state, _| state.allows_cli_changes()),
        "off while control from the command line is"
    );

    app.click(CONTROL_SELECTOR);
    assert_eq!(
        refusal(ask(&mut app, start(None, None, false))),
        Code::ChangesOff
    );
    app.click(CONTROL_CHANGES_SELECTOR);
    assert!(app.read(|state, _| state.allows_cli_changes()));
    assert_eq!(
        app.saved_settings()
            .map(|settings| (settings.allow_cli_control, settings.allow_cli_changes)),
        Some((true, true))
    );
    assert!(data(ask(&mut app, start(None, None, false)))["preview"].is_object());

    app.click(CONTROL_SELECTOR);
    assert_eq!(
        app.saved_settings()
            .map(|settings| (settings.allow_cli_control, settings.allow_cli_changes)),
        Some((false, false)),
        "turning control off takes starting and stopping back"
    );
    app.click(CONTROL_SELECTOR);
    assert_eq!(
        refusal(ask(&mut app, start(None, None, false))),
        Code::ChangesOff,
        "turned on again, it reads only until starting and stopping are allowed again too"
    );
}

#[gpui::test]
fn a_preview_says_what_would_happen_and_changes_nothing(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::single()));
    let preview = data(ask(&mut app, start(Some(Accelerator::T4), None, false)));
    assert!(preview["now"].is_u64());
    let preview = &preview["preview"];
    assert_eq!(preview["account"]["email"], "ada@example.com");
    assert!(preview["account"]["id"].is_string());
    let name = preview["name"].as_str().expect("a name").to_owned();
    assert!(name.starts_with("t4-"), "{name}");
    assert_eq!(preview["accelerator"], "T4");
    assert!(
        preview["estimated_rate_cu_per_hour"]
            .as_f64()
            .is_some_and(|rate| rate > 0.0)
    );
    assert_eq!(preview["rate_source"], "published");
    assert_eq!(preview["policy"]["max_runtime"], 7200, "{preview}");
    assert_eq!(preview["attempts"], 3);
    assert_eq!(preview["session_limit"]["limit"], 2);
    assert_eq!(preview["session_limit"]["counted"], serde_json::json!([]));
    assert_eq!(preview["session_limit"]["allows"], true);
    assert_eq!(
        data(ask(&mut app, start(Some(Accelerator::T4), None, false)))["preview"]["name"],
        name.as_str(),
        "the name it would get stays the same until it is used"
    );
    assert_eq!(
        data(ask(&mut app, start(Some(Accelerator::None), None, false)))["preview"]["min_vram_mib"],
        Value::Null
    );
    assert!(!app.read(|state, _| state.is_provisioning()));
    assert!(app.read(|state, _| state.new_session().is_none() && state.overlay().is_none()));
    app.settle();
    assert!(listed(&mut app).is_empty());
    assert!(app.toasts().is_empty());

    // Started under the name the preview showed.
    let (started, _) = followed(ask(&mut app, start(Some(Accelerator::T4), None, true)));
    assert_eq!(started["runtime"]["name"], name.as_str());
}

#[gpui::test]
fn a_stop_preview_names_the_runtime_and_what_ends_with_it(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::demo()));
    let preview = data(ask(&mut app, stop("cpu-0b5e", None, false)));
    let preview = &preview["preview"];
    assert_eq!(preview["runtime"]["label"], "cpu-0b5e");
    assert_eq!(preview["runtime"]["account_email"], "lin@example.com");
    assert_eq!(preview["runtime"]["on_active_account"], false);
    assert_eq!(preview["ends"], serde_json::json!(["billing", "files"]));
    let stopping = app.read(|state, _| {
        state.runtimes().ready().is_some_and(|view| {
            view.runtimes
                .iter()
                .any(|runtime| state.is_stopping(&runtime.id))
        })
    });
    assert!(!stopping, "a preview stops nothing");
    app.settle();
    assert!(listed_names(&mut app).contains(&"cpu-0b5e".to_owned()));
}

#[gpui::test]
fn a_start_shows_in_the_window_and_ends_in_a_listed_runtime(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::single()));
    on_sessions(&mut app);
    let (started, id) = followed(ask(
        &mut app,
        start(Some(Accelerator::T4), Some("train-1"), true),
    ));
    assert_eq!(started["kind"], "start");
    assert_eq!(started["state"], "running");
    assert_eq!(started["cancelling"], false);
    assert_eq!(started["runtime"]["name"], "train-1");
    assert_eq!(started["runtime"]["accelerator"], "T4");
    assert_eq!(started["account"]["email"], "ada@example.com");
    assert!(started["started_at"].is_u64());
    assert_eq!(started["ended_at"], Value::Null);

    // The window's own start: its progress, the quit prompt and the status all know of it.
    assert!(app.read(|state, _| state.is_provisioning()));
    assert!(app.read(|state, _| state.billing().provisioning));
    assert!(matches!(
        app.read(|state, _| state.new_session().map(|session| session.phase.clone())),
        Some(ProvisionPhase::Running)
    ));
    assert!(
        app.read(|state, _| state.overlay().is_none()),
        "no dialog pops up"
    );
    let status = data(ask(&mut app, Command::Status));
    assert_eq!(status["activity"], serde_json::json!(["starting_runtime"]));
    let starting = toast_text(&mut app, keys::CTL_STARTING_TOAST, "train-1");
    assert!(app.toasts().contains(&starting));
    app.draw_now();
    assert!(
        app.shown(PROVISIONING_SELECTOR),
        "the Sessions page shows it"
    );
    let (running, _) = followed(ask(&mut app, operation(&id)));
    assert_eq!(running["state"], "running");

    app.settle();
    let (ended, _) = followed(ask(&mut app, operation(&id)));
    assert_eq!(ended["state"], "succeeded", "{ended}");
    assert_eq!(ended["outcome"]["name"], "train-1");
    assert!(ended["ended_at"].is_u64());
    assert!(
        ended["progress"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["event"] == "ready")),
        "{ended}"
    );
    assert_eq!(ended["error"], Value::Null);
    let runtime = listed(&mut app)
        .into_iter()
        .find(|runtime| runtime["id"]["name"] == "train-1")
        .expect("listed like any other");
    assert_eq!(runtime["managed"], true);
    assert_eq!(runtime["accelerator"], "T4");
    assert!(!app.read(|state, _| state.is_provisioning()));
}

#[gpui::test]
fn operation_ids_are_told_apart_from_failed_operations(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::single()));
    let (_, id) = followed(ask(&mut app, start(None, None, true)));
    app.settle();
    let (run, count) = id.rsplit_once('-').expect("a run and a count");
    assert_eq!(run.len(), 8, "{id}");
    let other_run = format!(
        "{}-{count}",
        if run == "00000000" {
            "ffffffff"
        } else {
            "00000000"
        }
    );
    for unknown in ["nope", "", other_run.as_str()] {
        assert_eq!(
            refusal(ask(&mut app, operation(unknown))),
            Code::UnknownOperation
        );
        assert_eq!(
            refusal(ask(&mut app, cancel(unknown, true))),
            Code::UnknownOperation
        );
    }
    assert_eq!(
        refusal(ask(&mut app, cancel(&id, true))),
        Code::OperationEnded,
        "a start that is done is not cancelled"
    );
}

#[gpui::test]
fn a_start_that_fails_is_a_failed_operation(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            not_entitled: vec![Accelerator::H100],
            ..MockFailures::default()
        },
        ..changes_allowed(CHANGES_ALLOWED, MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    let (_, id) = followed(ask(&mut app, start(Some(Accelerator::H100), None, true)));
    app.settle();
    let response = ask(&mut app, operation(&id));
    assert!(response.ok, "the operation is known; it failed");
    let (failed, _) = followed(response);
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["error"]["kind"], "not_entitled");
    assert!(listed(&mut app).is_empty());
}

#[gpui::test]
fn a_programs_stop_does_not_ask_for_a_star(cx: &mut TestAppContext) {
    // Due to ask: after the user's own stop with nothing left billing, the prompt would open.
    let settings = r#"{ "allow_cli_control": true, "allow_cli_changes": true, "ready_count": 5 }"#;
    let mut app = TestApp::with(cx, changes_allowed(settings, MockScenario::single()));
    let (_, id) = followed(ask(&mut app, start(None, Some("only-one"), true)));
    app.settle();
    assert_eq!(
        followed(ask(&mut app, operation(&id))).0["state"],
        "succeeded"
    );
    let (_, id) = followed(ask(&mut app, stop("only-one", None, true)));
    app.settle();
    assert_eq!(
        followed(ask(&mut app, operation(&id))).0["state"],
        "succeeded"
    );
    assert!(listed(&mut app).is_empty());
    assert!(app.read(|state, _| state.overlay().is_none()));

    // The same again, stopped in the window: then it asks.
    ask(&mut app, start(None, Some("another"), true));
    app.settle();
    let target = app.read(|state, _| {
        state.runtimes().ready().expect("listed").runtimes[0]
            .id
            .clone()
    });
    app.update(|state, cx| {
        state.request_stop(target, cx);
        state.confirm_stop(cx);
    });
    assert!(matches!(
        app.read(|state, _| state.overlay().cloned()),
        Some(Overlay::Feedback)
    ));
}

#[gpui::test]
fn cancelling_a_start_leaves_no_runtime_behind(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::single()));
    on_sessions(&mut app);
    let (_, id) = followed(ask(&mut app, start(Some(Accelerator::A100), None, true)));
    let preview = data(ask(&mut app, cancel(&id, false)));
    assert_eq!(preview["preview"]["operation"]["id"], id.as_str());
    assert!(
        !app.read(|state, _| state.is_cancelling_provision()),
        "a preview cancels nothing"
    );
    let (cancelling, _) = followed(ask(&mut app, cancel(&id, true)));
    assert_eq!(cancelling["state"], "running");
    assert_eq!(cancelling["cancelling"], true);
    app.draw_now();
    assert!(
        app.shown(PROVISIONING_SELECTOR),
        "the window shows it winding down"
    );

    app.settle();
    let (ended, _) = followed(ask(&mut app, operation(&id)));
    assert_eq!(ended["state"], "cancelled", "{ended}");
    assert_eq!(ended["error"], Value::Null);
    let left = app.mock.runtimes(&CancelToken::new()).expect("listing");
    assert!(left.runtimes.is_empty(), "nothing billed is left: {left:?}");
    assert!(listed(&mut app).is_empty());
}

#[gpui::test]
fn a_cancel_in_the_window_shows_on_the_operation(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::single()));
    // Wait 1 is in `create`, before the VM exists; wait 2 is the hardware probe, with a VM
    // allocated: the window's Cancel lands there.
    app.cancel_switch.arm(2);
    let (_, id) = followed(ask(&mut app, start(Some(Accelerator::A100), None, true)));
    app.settle();
    let (ended, _) = followed(ask(&mut app, operation(&id)));
    assert_eq!(ended["state"], "cancelled", "{ended}");
    assert!(
        ended["progress"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["event"] == "releasing")),
        "the runtime it got was given back: {ended}"
    );
    let left = app.mock.runtimes(&CancelToken::new()).expect("listing");
    assert!(left.runtimes.is_empty(), "nothing billed is left: {left:?}");
}

#[gpui::test]
fn stop_reaches_every_runtime_sessions_lists(cx: &mut TestAppContext) {
    let mut app = TestApp::with(
        cx,
        changes_allowed(CHANGES_ALLOWED_ROOMY, MockScenario::demo()),
    );
    // One name on two accounts: the active one gets a runtime with the other's name.
    let (_, id) = followed(ask(
        &mut app,
        start(Some(Accelerator::None), Some("cpu-0b5e"), true),
    ));
    app.settle();
    assert_eq!(
        followed(ask(&mut app, operation(&id))).0["state"],
        "succeeded"
    );
    assert_eq!(
        refusal(ask(&mut app, stop("cpu-0b5e", None, true))),
        Code::AmbiguousRuntime,
        "never a guess"
    );
    assert_eq!(
        refusal(ask(&mut app, stop("nothing-here", None, true))),
        Code::NoSuchRuntime
    );
    assert_eq!(
        refusal(ask(
            &mut app,
            stop("a100-7f3a", Some("lin@example.com"), true)
        )),
        Code::NoSuchRuntime,
        "not on that account"
    );

    // On an account that is not the active one, named by its email.
    let (stopping, id) = followed(ask(
        &mut app,
        stop("cpu-0b5e", Some("lin@example.com"), true),
    ));
    assert_eq!(stopping["kind"], "stop");
    assert_eq!(stopping["state"], "running");
    assert_eq!(stopping["account"]["email"], "lin@example.com");
    assert_eq!(stopping["runtime"]["label"], "cpu-0b5e");
    let stopping_toast = toast_text(&mut app, keys::CTL_STOPPING_TOAST, "cpu-0b5e");
    assert!(app.toasts().contains(&stopping_toast));
    assert_eq!(
        refusal(ask(
            &mut app,
            stop("cpu-0b5e", Some("lin@example.com"), true)
        )),
        Code::AlreadyStopping
    );
    assert_eq!(
        refusal(ask(&mut app, cancel(&id, true))),
        Code::NotCancellable
    );
    app.settle();
    assert_eq!(
        followed(ask(&mut app, operation(&id))).0["state"],
        "succeeded"
    );
    let runtimes = listed(&mut app);
    assert!(
        !runtimes
            .iter()
            .any(|runtime| runtime["account_email"] == "lin@example.com"),
        "{runtimes:?}"
    );
    let ended = data(ask(&mut app, Command::Sessions))["ended"][0].clone();
    assert_eq!(ended["label"], "cpu-0b5e");
    assert_eq!(ended["reason"], "command_line");

    // Opened in the browser: no name, so named by its endpoint.
    let endpoint = runtimes
        .iter()
        .find(|runtime| runtime["id"]["name"].is_null())
        .and_then(|runtime| runtime["id"]["endpoint"].as_str())
        .expect("the runtime opened in the browser")
        .to_owned();
    let (_, id) = followed(ask(&mut app, stop(&endpoint, None, true)));
    // By its session name, now that it is the only one.
    let (_, by_name) = followed(ask(&mut app, stop("cpu-0b5e", None, true)));
    app.settle();
    for id in [id, by_name] {
        assert_eq!(
            followed(ask(&mut app, operation(&id))).0["state"],
            "succeeded"
        );
    }
    let left = listed(&mut app);
    assert!(
        left.iter()
            .all(|runtime| !runtime["id"]["name"].is_null() && runtime["id"]["name"] != "cpu-0b5e"),
        "{left:?}"
    );
    assert_eq!(listed_names(&mut app), ["a100-7f3a", "t4-91c2"]);
}

#[gpui::test]
fn a_start_is_refused_while_one_runs_or_the_form_is_open(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::single()));
    let (_, id) = followed(ask(&mut app, start(None, None, true)));
    for yes in [false, true] {
        assert_eq!(
            refusal(ask(&mut app, start(None, None, yes))),
            Code::AlreadyStarting,
            "refused, not queued"
        );
    }
    app.settle();
    assert_eq!(
        followed(ask(&mut app, operation(&id))).0["state"],
        "succeeded"
    );
    assert_eq!(listed(&mut app).len(), 1, "only the first one started");

    on_sessions(&mut app);
    app.click(NEW_SESSION_SELECTOR);
    assert!(matches!(
        app.read(|state, _| state.overlay().cloned()),
        Some(Overlay::NewSession)
    ));
    let typed = app.read(|state, _| state.new_session().map(|session| session.spec.clone()));
    assert_eq!(
        refusal(ask(&mut app, start(None, None, true))),
        Code::FormOpen,
        "what is being filled in stays"
    );
    assert_eq!(
        app.read(|state, _| state.new_session().map(|session| session.spec.clone())),
        typed
    );
}

#[gpui::test]
fn a_start_leaves_a_dialog_that_holds_what_was_entered(cx: &mut TestAppContext) {
    let options = Options {
        failures: MockFailures {
            not_entitled: vec![Accelerator::H100],
            ..MockFailures::default()
        },
        ..changes_allowed(CHANGES_ALLOWED_ROOMY, MockScenario::single())
    };
    let mut app = TestApp::with(cx, options);
    let phase = |app: &mut TestApp<'_>| {
        app.read(|state, _| state.new_session().map(|session| session.phase.clone()))
    };
    on_sessions(&mut app);
    app.click(NEW_SESSION_SELECTOR);
    app.update(|state, cx| {
        state.edit_new_session(|spec| spec.accelerator = Accelerator::H100, cx);
        state.start_provision("mine".into(), cx);
    });
    assert!(matches!(phase(&mut app), Some(ProvisionPhase::Failed(_))));
    assert_eq!(
        refusal(ask(&mut app, start(None, None, true))),
        Code::FormOpen,
        "why it failed, and the values to try again with, stay"
    );
    assert!(matches!(phase(&mut app), Some(ProvisionPhase::Failed(_))));

    app.set_failures(MockFailures::default());
    app.update(|state, cx| state.start_provision("mine".into(), cx));
    assert!(matches!(phase(&mut app), Some(ProvisionPhase::Ready(_))));
    assert!(matches!(
        app.read(|state, _| state.overlay().cloned()),
        Some(Overlay::NewSession)
    ));
    let (_, id) = followed(ask(&mut app, start(None, None, true)));
    app.settle();
    assert_eq!(
        followed(ask(&mut app, operation(&id))).0["state"],
        "succeeded",
        "a dialog that shows the runtime it started holds nothing to keep"
    );
}

#[gpui::test]
fn a_start_is_refused_at_the_limit_while_accounts_change_or_the_app_closes(
    cx: &mut TestAppContext,
) {
    let mut app = TestApp::with(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::demo()));
    let response = ask(&mut app, start(None, None, false));
    let message = response
        .error
        .as_ref()
        .and_then(|failure| failure.message.clone())
        .unwrap_or_default();
    assert_eq!(refusal(response), Code::SessionLimitReached);
    assert!(
        message.contains("a100-7f3a") && message.contains("t4-91c2"),
        "{message}"
    );

    let lin = app.read(|state, _| {
        state
            .accounts()
            .ready()
            .and_then(|view| {
                view.accounts
                    .iter()
                    .find(|account| account.email.as_deref() == Some("lin@example.com"))
                    .map(|account| account.id.clone())
            })
            .expect("the second account")
    });
    // The switch dialog open, not confirmed yet: a start could otherwise run under the switch.
    app.update(|state, cx| state.request_switch(lin.clone(), cx));
    assert_eq!(
        refusal(ask(&mut app, start(None, None, true))),
        Code::AccountChanging
    );
    assert!(
        data(ask(&mut app, stop("a100-7f3a", None, false)))["preview"].is_object(),
        "a stop does not depend on which account is active"
    );
    app.update(|state, cx| state.close_dialog(cx));

    let state = app.state.clone();
    app.cx.update(|_, cx| {
        state.update(cx, |state, cx| {
            state.request_switch(lin, cx);
            state.confirm_switch(cx);
        })
    });
    assert_eq!(
        refusal(ask(&mut app, start(None, None, true))),
        Code::AccountChanging
    );
    assert_eq!(
        refusal(ask(&mut app, stop("a100-7f3a", None, true))),
        Code::AccountChanging
    );
    app.settle();

    app.update(|state, cx| state.request_quit(cx));
    assert!(matches!(
        app.read(|state, _| state.overlay().cloned()),
        Some(Overlay::Quit(_))
    ));
    assert_eq!(
        refusal(ask(&mut app, start(None, None, true))),
        Code::Quitting
    );
    assert_eq!(
        refusal(ask(&mut app, stop("a100-7f3a", None, true))),
        Code::Quitting
    );
}

#[gpui::test]
fn a_start_needs_an_active_account(cx: &mut TestAppContext) {
    let mut app = TestApp::with(
        cx,
        changes_allowed(CHANGES_ALLOWED, MockScenario::signed_out()),
    );
    assert_eq!(
        refusal(ask(&mut app, start(None, None, true))),
        Code::NoActiveAccount
    );
    assert!(!app.read(|state, _| state.is_provisioning()));
}

#[gpui::test]
fn what_is_not_loaded_yet_is_not_ready_rather_than_absent(cx: &mut TestAppContext) {
    let launch = Launch::new(cx, changes_allowed(CHANGES_ALLOWED, MockScenario::demo()));
    let answer = |cx: &mut TestAppContext, command| {
        let state = launch.state.clone();
        cx.update(|cx| control::answer(command, &state, cx))
    };
    assert_eq!(refusal(answer(cx, start(None, None, true))), Code::NotReady);
    assert_eq!(
        refusal(answer(cx, stop("a100-7f3a", None, true))),
        Code::NotReady,
        "not that no such runtime exists"
    );
}
