//! The deployments list and Google Drive over the headless mock: a started deployment is
//! recorded (secrets left out), deployed again with the same settings, removed; a deployment
//! with Drive on mounts it before the script starts.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use colab_playground::{
    deployments::{DEPLOYMENTS_FILE, Deployment},
    route::Route,
    state::{DriveStep, PluginView, RunPhase},
    ui::pages::{
        deployments::{
            DEPLOYMENTS, DEPLOYMENTS_EMPTY, deployment_selector, forget_deployment_selector,
            redeploy_selector,
        },
        plugins::{RUN_DRIVE, RUN_DRIVE_MOUNTED, RUN_START},
    },
};
use colab_playground_core::{
    backend::Backend,
    mock::MockScenario,
    plugins::{InputValue, drive_dir},
};
use gpui::TestAppContext;

use crate::support::{Options, TestApp};

const REPO: &str = "unsloth/Qwen3-0.6B-GGUF";

fn open_form(app: &mut TestApp<'_>) {
    app.click(Route::Plugins.nav_selector());
    app.click("plugin-run llama-cpp");
    assert_eq!(app.read(|state, _| state.plugin_view()), PluginView::Form);
}

fn field_text(app: &mut TestApp<'_>, id: &str) -> String {
    app.read_root(|root, cx| {
        root.inputs()
            .field(id)
            .expect("a field")
            .read(cx)
            .text()
            .to_owned()
    })
}

fn deployments(app: &mut TestApp<'_>) -> Vec<Deployment> {
    app.read(|state, _| state.deployments().to_vec())
}

fn saved(app: &TestApp<'_>) -> Vec<Deployment> {
    let file = app
        .config_dir()
        .expect("a config dir")
        .join(DEPLOYMENTS_FILE);
    serde_json::from_str(&std::fs::read_to_string(file).expect("written")).expect("valid")
}

#[gpui::test]
fn a_deployment_is_recorded_deployed_again_and_removed(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    open_form(&mut app);
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    app.fill(
        |inputs| inputs.field("api_key").expect("the key field"),
        "sk-never-saved",
    );
    app.fill(
        |inputs| inputs.field("ctx").expect("the context field"),
        "8192",
    );
    app.click(RUN_START);
    assert!(matches!(
        app.read(|state, _| state.plugin_run().map(|r| r.phase.clone())),
        Some(RunPhase::Ready(_))
    ));

    let list = deployments(&mut app);
    assert_eq!(list.len(), 1);
    let record = list[0].clone();
    assert_eq!(
        record.inputs.get("repo"),
        Some(&InputValue::Text(REPO.into()))
    );
    assert!(
        !record.inputs.contains_key("api_key"),
        "secrets are not kept"
    );
    assert_eq!(saved(&app), list, "the list is saved");
    let text = std::fs::read_to_string(app.config_dir().unwrap().join(DEPLOYMENTS_FILE)).unwrap();
    assert!(!text.contains("sk-never-saved"));

    // The Deployments page, a sidebar entry of its own, lists it.
    app.click(Route::Deployments.nav_selector());
    assert_eq!(app.route(), Route::Deployments);
    assert!(app.shown(DEPLOYMENTS));
    assert!(app.shown(&deployment_selector(&record.id)));

    // Deploying again opens the form on the Plugins page with the same model and context, and
    // no key.
    app.click(&redeploy_selector(&record.id));
    assert_eq!(app.route(), Route::Plugins);
    assert_eq!(app.read(|state, _| state.plugin_view()), PluginView::Form);
    assert_eq!(field_text(&mut app, "repo"), REPO);
    assert_eq!(field_text(&mut app, "ctx"), "8192");
    assert_eq!(field_text(&mut app, "api_key"), "");

    // Removing it empties the list, and the page says nothing was deployed.
    app.click(Route::Deployments.nav_selector());
    app.click(&forget_deployment_selector(&record.id));
    assert!(deployments(&mut app).is_empty());
    assert!(saved(&app).is_empty());
    assert!(!app.shown(DEPLOYMENTS));
    assert!(app.shown(DEPLOYMENTS_EMPTY));
}

#[gpui::test]
fn a_deployment_with_drive_mounts_it_first(cx: &mut TestAppContext) {
    let mut app = TestApp::with(cx, Options::scenario(MockScenario::single()));
    open_form(&mut app);
    app.fill(
        |inputs| inputs.field("repo").expect("the model field"),
        REPO,
    );
    assert!(
        app.shown(RUN_DRIVE),
        "the plugin can keep its files on Drive"
    );
    assert!(
        !app.read(|state, _| state.run_form().unwrap().drive),
        "off unless turned on"
    );
    app.click(RUN_DRIVE);
    assert!(app.read(|state, _| state.run_form().unwrap().drive));

    // The mock waits for consent the way the CLI does. The test executor runs the backend
    // call on this thread, so the consent comes from another one, as a click on Continue would
    // reach the backend.
    let done = Arc::new(AtomicBool::new(false));
    let granter = {
        let (mock, done) = (app.mock.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                mock.confirm_drive_consent();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
    };
    app.click(RUN_START);
    done.store(true, Ordering::SeqCst);
    granter.join().unwrap();

    let run = app
        .read(|state, _| state.plugin_run().cloned())
        .expect("a run");
    assert!(matches!(run.phase, RunPhase::Ready(_)), "{:?}", run.phase);
    assert_eq!(
        run.drive,
        DriveStep::Mounted {
            dir: drive_dir("llama-cpp")
        }
    );
    assert!(app.shown(RUN_DRIVE_MOUNTED), "the folder is shown");
    assert!(deployments(&mut app)[0].drive, "the record keeps Drive on");
}
