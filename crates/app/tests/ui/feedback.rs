//! Asking for a star or feedback over the headless mock: when it comes, what each choice
//! opens, and when it stops asking.

use colab_playground::{
    route::Route,
    state::{
        ASK_AGAIN_AFTER, MOST_ASKS, Overlay, PROJECT_PAGE, ProvisionPhase, READY_BEFORE_ASKING,
        RunPhase, SPONSOR_PAGE, feedback_page, runtime_label,
    },
    ui::{
        overlays::{
            FEEDBACK_GIVE, FEEDBACK_PROMPT, FEEDBACK_STAR, NEW_SESSION_START, STOP_CONFIRM,
        },
        pages::{
            plugins::RUN_START,
            sessions::{NEW_SESSION_SELECTOR, stop_selector},
            settings::{FEEDBACK_LINK_SELECTOR, SPONSOR_LINK_SELECTOR},
        },
    },
};
use colab_playground_core::{mock::MockScenario, sessions::Accelerator};
use gpui::TestAppContext;

use crate::support::{ConfigDir, Options, TestApp};

/// A single account with nothing running, where `ready` runtimes had come up before.
fn used(ready: u32) -> Options {
    Options {
        config: ConfigDir::TempWith(format!(r#"{{ "ready_count": {ready} }}"#)),
        ..Options::scenario(MockScenario::single())
    }
}

/// A New session with the defaults, until it is ready; returns its name.
fn start_session(app: &mut TestApp<'_>) -> String {
    app.click(Route::Sessions.nav_selector());
    app.click(NEW_SESSION_SELECTOR);
    app.click(NEW_SESSION_START);
    let phase = app.read(|state, _| state.new_session().map(|session| session.phase.clone()));
    let Some(ProvisionPhase::Ready(outcome)) = phase else {
        panic!("expected a ready session, got {phase:?}");
    };
    app.click("overlay-cancel");
    outcome.name
}

/// "Stop and end billing" on the runtime's card, confirmed.
fn stop(app: &mut TestApp<'_>, label: &str) {
    app.click(Route::Sessions.nav_selector());
    app.click(&stop_selector(label));
    app.click(STOP_CONFIRM);
}

fn start_and_stop(app: &mut TestApp<'_>) {
    let name = start_session(app);
    stop(app, &name);
}

fn asking(app: &mut TestApp<'_>) -> bool {
    let open = app.read(|state, _| matches!(state.overlay(), Some(Overlay::Feedback)));
    assert_eq!(open, app.shown(FEEDBACK_PROMPT), "drawn exactly while open");
    open
}

#[gpui::test]
fn it_asks_after_a_few_runtimes_at_the_pause_after_a_stop(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    for _ in 1..READY_BEFORE_ASKING {
        start_and_stop(&mut app);
        assert!(!asking(&mut app), "too early");
    }
    let name = start_session(&mut app);
    assert!(!asking(&mut app), "not while it runs");
    assert_eq!(
        app.saved_settings().map(|s| s.ready_count),
        Some(READY_BEFORE_ASKING)
    );
    stop(&mut app, &name);
    assert!(asking(&mut app), "the user just ended their work");
    let saved = app.saved_settings().expect("saved");
    assert_eq!(saved.feedback_asked, 1);
    assert!(saved.feedback_asked_at.is_some());
    assert!(!saved.feedback_answered);
}

#[gpui::test]
fn a_deployment_counts_too(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, used(READY_BEFORE_ASKING - 1));
    app.click(Route::Plugins.nav_selector());
    app.click("plugin-run llama-cpp");
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        "unsloth/Qwen3-0.6B-GGUF",
    );
    app.click(RUN_START);
    let phase = app.read(|state, _| state.plugin_run().map(|run| run.phase.clone()));
    let Some(RunPhase::Ready(outcome)) = phase else {
        panic!("expected a ready run, got {phase:?}");
    };
    assert_eq!(
        app.saved_settings().map(|s| s.ready_count),
        Some(READY_BEFORE_ASKING)
    );
    assert!(!asking(&mut app), "not while the model is served");
    stop(&mut app, &outcome.session);
    assert!(asking(&mut app));
}

#[gpui::test]
fn star_opens_the_project_page_and_ends_the_asks(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, used(READY_BEFORE_ASKING));
    start_and_stop(&mut app);
    assert!(asking(&mut app));
    app.click(FEEDBACK_STAR);
    assert_eq!(app.opened_url().as_deref(), Some(PROJECT_PAGE));
    assert!(!asking(&mut app), "it closes");
    assert_eq!(
        app.saved_settings().map(|s| s.feedback_answered),
        Some(true)
    );

    app.clock.advance(ASK_AGAIN_AFTER * 2);
    start_and_stop(&mut app);
    assert!(!asking(&mut app), "never again after an answer");
}

#[gpui::test]
fn give_feedback_opens_a_new_issue(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, used(READY_BEFORE_ASKING));
    start_and_stop(&mut app);
    app.click(FEEDBACK_GIVE);
    let opened = app.opened_url().expect("a page was opened");
    assert_eq!(opened, feedback_page());
    assert!(opened.starts_with(&format!("{PROJECT_PAGE}/issues/new")));
    assert!(!asking(&mut app));
    assert_eq!(
        app.saved_settings().map(|s| s.feedback_answered),
        Some(true)
    );
}

#[gpui::test]
fn not_now_asks_once_more_a_while_later_and_then_stops(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, used(READY_BEFORE_ASKING));
    start_and_stop(&mut app);
    assert!(asking(&mut app));
    assert!(!app.read(|state, _| state.feedback_asked_for_the_last_time()));
    app.click("overlay-cancel");
    assert!(!asking(&mut app));
    assert!(app.opened_url().is_none(), "nothing was opened");

    start_and_stop(&mut app);
    assert!(!asking(&mut app), "not again so soon");

    app.clock.advance(ASK_AGAIN_AFTER);
    start_and_stop(&mut app);
    assert!(asking(&mut app), "asked a second time");
    assert!(
        app.read(|state, _| state.feedback_asked_for_the_last_time()),
        "the way out says it will not ask again"
    );
    app.press("escape");
    assert!(!asking(&mut app), "Escape closes it as well");

    app.clock.advance(ASK_AGAIN_AFTER * 2);
    start_and_stop(&mut app);
    assert!(!asking(&mut app), "and that was the last time");
    let saved = app.saved_settings().expect("saved");
    assert_eq!(
        (saved.feedback_asked, saved.feedback_answered),
        (MOST_ASKS, false)
    );
}

#[gpui::test]
fn it_waits_until_nothing_runs_on_any_account(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith(format!(r#"{{ "ready_count": {READY_BEFORE_ASKING} }}"#)),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    // Two sessions and a runtime opened in the browser on the active account, one session on
    // the other account.
    for label in ["a100-7f3a", "t4-91c2", "L4"] {
        stop(&mut app, label);
        assert!(!asking(&mut app), "{label}: others still bill");
    }
    stop(&mut app, "cpu-0b5e");
    assert!(asking(&mut app), "the last one ended");
}

#[gpui::test]
fn it_decides_on_the_list_fetched_after_the_stop(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, used(READY_BEFORE_ASKING));
    let name = start_session(&mut app);
    let account = app
        .read(|state, _| {
            let view = state.runtimes().ready()?;
            view.runtimes
                .iter()
                .find(|runtime| runtime.id.name.as_deref() == Some(name.as_str()))
                .map(|runtime| runtime.id.account.clone())
        })
        .expect("the new session is listed");
    // Opened in the browser since the last listing: the list shown lacks it while it bills,
    // as it lacks a runtime whose start finished during the stop.
    let elsewhere = app.mock.add_unmanaged_runtime(&account, Accelerator::T4);
    stop(&mut app, &name);
    assert!(
        !asking(&mut app),
        "the runtime opened elsewhere still bills"
    );

    let label = app
        .read(|state, _| {
            let view = state.runtimes().ready()?;
            view.runtimes
                .iter()
                .find(|runtime| runtime.id.endpoint == elsewhere)
                .map(runtime_label)
        })
        .expect("listed after the stop");
    stop(&mut app, &label);
    assert!(asking(&mut app), "nothing bills any more");
}

#[gpui::test]
fn it_asks_after_a_stop_that_returned_during_a_listing(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, used(READY_BEFORE_ASKING));
    let name = start_session(&mut app);
    app.click(Route::Sessions.nav_selector());
    app.click(&stop_selector(&name));
    // A listing and the stop at once: whichever returns first, the ask comes once a listing
    // that began after the stop is back.
    app.update(|state, cx| {
        state.confirm_stop(cx);
        state.load_runtimes(cx);
    });
    assert!(asking(&mut app));
}

#[gpui::test]
fn about_offers_feedback_at_any_time(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    app.click(Route::Settings.nav_selector());
    app.click(FEEDBACK_LINK_SELECTOR);
    assert_eq!(app.opened_url(), Some(feedback_page()));
    assert!(
        !app.saved_settings().is_some_and(|s| s.feedback_answered),
        "only an answer to the prompt ends the asks"
    );
}

#[gpui::test]
fn about_opens_the_sponsor_page(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    app.click(Route::Settings.nav_selector());
    app.click(SPONSOR_LINK_SELECTOR);
    assert_eq!(app.opened_url().as_deref(), Some(SPONSOR_PAGE));
    assert!(
        !app.saved_settings().is_some_and(|s| s.feedback_answered),
        "only an answer to the prompt ends the asks"
    );
}
