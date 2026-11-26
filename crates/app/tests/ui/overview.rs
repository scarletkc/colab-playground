//! The first-run checklist and the CLI install, over the headless mock.

use colab_playground::{
    platform::{EnvironmentKind, environment_kind},
    route::Route,
    state::{EnvironmentStep, InstallPhase, Load, Op, StepState, checklist},
    ui::pages::{
        defaults::field_selector,
        overview::{
            CHECK_AGAIN_SELECTOR, INSTALL_LOG_SELECTOR, INSTALL_NOT_FOUND_SELECTOR,
            INSTALL_OPEN_SETTINGS_SELECTOR, INSTALL_SELECTOR, SIGN_IN_SELECTOR, SUMMARY_SELECTOR,
            step_selector,
        },
        settings::LANGUAGE_SELECTOR,
    },
};
use colab_playground_core::{
    ErrorKind,
    accounts::AccountsView,
    host::ProbeReport,
    mock::{MockFailures, MockScenario},
};
use gpui::TestAppContext;

use crate::support::{Options, TestApp};

fn first_step() -> EnvironmentStep {
    match environment_kind() {
        EnvironmentKind::Wsl => EnvironmentStep::Wsl,
        EnvironmentKind::ShellTools => EnvironmentStep::ShellTools,
    }
}

fn cli_missing(install_fails: bool) -> Options {
    Options {
        failures: MockFailures {
            cli_missing: true,
            install_fails,
            ..MockFailures::default()
        },
        ..Options::scenario(MockScenario::signed_out())
    }
}

fn step_state(app: &mut TestApp<'_>, step: EnvironmentStep) -> StepState {
    app.read(|state, _| state.checklist().state(step).cloned())
        .expect("the step is on the checklist")
}

#[gpui::test]
fn a_missing_cli_installs_and_the_checklist_moves_on(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, cli_missing(false));
    assert_eq!(step_state(&mut app, first_step()), StepState::Done);
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Todo);
    assert_eq!(
        step_state(&mut app, EnvironmentStep::SignIn),
        StepState::Waiting
    );
    assert!(app.shown(&step_selector(EnvironmentStep::Cli, &StepState::Todo)));

    app.click(INSTALL_SELECTOR);
    assert_eq!(
        app.read(|state, _| state.install().phase.clone()),
        InstallPhase::Done
    );
    let log: Vec<String> = app.read(|state, _| {
        state
            .install()
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
    });
    assert!(
        log.iter()
            .any(|line| line.contains("Installed 1 executable: colab")),
        "{log:?}"
    );
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Done);
    assert_eq!(
        step_state(&mut app, EnvironmentStep::SignIn),
        StepState::Todo
    );
    assert!(app.shown(SIGN_IN_SELECTOR), "signing in is offered next");
    assert!(!app.shown(SUMMARY_SELECTOR));
}

#[gpui::test]
fn a_failed_install_says_so_keeps_the_log_and_can_run_again(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, cli_missing(true));
    app.click(INSTALL_SELECTOR);
    let phase = app.read(|state, _| state.install().phase.clone());
    let InstallPhase::Failed(error) = phase else {
        panic!("expected a failure, got {phase:?}");
    };
    assert_eq!(error.kind, ErrorKind::InstallFailed);
    assert!(app.shown(INSTALL_LOG_SELECTOR), "the log stays for reading");
    assert!(app.shown("error-install"), "the failure is explained");
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Todo);

    app.set_failures(MockFailures::default());
    app.click(INSTALL_SELECTOR);
    assert_eq!(
        app.read(|state, _| state.install().phase.clone()),
        InstallPhase::Done
    );
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Done);
}

#[gpui::test]
fn an_install_that_still_misses_colab_points_to_settings(cx: &mut TestAppContext) {
    let mut app = TestApp::with(
        cx,
        Options {
            failures: MockFailures {
                cli_missing: true,
                install_off_path: true,
                ..MockFailures::default()
            },
            ..Options::scenario(MockScenario::signed_out())
        },
    );
    assert!(!app.shown(INSTALL_NOT_FOUND_SELECTOR));

    app.click(INSTALL_SELECTOR);
    assert_eq!(
        app.read(|state, _| state.install().phase.clone()),
        InstallPhase::Done
    );
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Todo);
    assert!(
        app.shown(INSTALL_NOT_FOUND_SELECTOR),
        "the step says why it is not done"
    );
    assert!(
        app.shown(INSTALL_LOG_SELECTOR),
        "the log may name the folder"
    );
    assert!(
        app.shown(INSTALL_SELECTOR),
        "another method can still be tried"
    );

    app.click(INSTALL_OPEN_SETTINGS_SELECTOR);
    assert_eq!(app.route(), Route::Settings);
    assert!(
        host_comes_first(&mut app),
        "the setting the alert names is the first one"
    );

    // The user put the folder on PATH meanwhile.
    app.set_failures(MockFailures::default());
    app.click(Route::Overview.nav_selector());
    app.click(CHECK_AGAIN_SELECTOR);
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Done);
    assert!(!app.shown(INSTALL_NOT_FOUND_SELECTOR));

    app.click(Route::Settings.nav_selector());
    assert!(
        !host_comes_first(&mut app),
        "from the sidebar, Settings keeps its order"
    );
}

/// Whether the colab path field sits above the language select on Settings.
fn host_comes_first(app: &mut TestApp<'_>) -> bool {
    let path = app.bounds(&field_selector("colab_path"));
    let language = app.bounds(LANGUAGE_SELECTOR);
    let (path, language) = path.zip(language).expect("both are drawn");
    path.origin.y < language.origin.y
}

#[gpui::test]
fn cancelling_an_install_stops_it(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, cli_missing(false));
    app.update(|state, cx| {
        state.start_install(cx);
        assert!(!state.is_cancelling(Op::Install));
        state.cancel_install(cx);
        assert!(
            state.is_cancelling(Op::Install),
            "the button reads Cancelling… until the call stops"
        );
    });
    assert!(!app.read(|state, _| state.is_cancelling(Op::Install)));
    assert_eq!(
        app.read(|state, _| state.install().phase.clone()),
        InstallPhase::Cancelled
    );
    assert_eq!(step_state(&mut app, EnvironmentStep::Cli), StepState::Todo);
    assert!(app.shown(INSTALL_SELECTOR), "it can be started again");
}

#[gpui::test]
fn once_set_up_the_overview_shows_the_summary(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    assert!(app.read(|state, _| state.checklist().complete()));
    assert!(app.shown(SUMMARY_SELECTOR));
    assert!(
        app.shown("unmanaged-warning"),
        "the demo has a runtime opened in the browser"
    );
}

#[test]
fn the_first_step_is_wsl_on_windows_and_shell_tools_elsewhere() {
    let ready = |report: ProbeReport| Load::Ready(report);
    let accounts: Load<AccountsView> = Load::Idle;
    let no_distro = ProbeReport {
        wsl_installed: true,
        ..ProbeReport::default()
    };
    let wsl = checklist(EnvironmentKind::Wsl, &ready(no_distro.clone()), &accounts);
    assert_eq!(wsl.steps[0].step, EnvironmentStep::Wsl);
    assert_eq!(
        wsl.steps[0].state,
        StepState::Todo,
        "WSL without a distribution"
    );
    assert_eq!(wsl.steps[1].state, StepState::Waiting);

    let shell = checklist(
        EnvironmentKind::ShellTools,
        &ready(ProbeReport {
            bash: Some("/bin/bash".into()),
            ..ProbeReport::default()
        }),
        &accounts,
    );
    assert_eq!(shell.steps[0].step, EnvironmentStep::ShellTools);
    assert_eq!(shell.steps[0].state, StepState::Done);
    assert_eq!(shell.steps[1].state, StepState::Todo, "no colab yet");
    assert!(
        !shell
            .steps
            .iter()
            .any(|step| step.step == EnvironmentStep::Wsl),
        "no WSL step off Windows"
    );

    let checking = checklist(EnvironmentKind::Wsl, &Load::Loading, &accounts);
    assert_eq!(checking.steps[0].state, StepState::Checking);
}
