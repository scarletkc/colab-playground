use colab_playground::{route::Route, ui::sidebar};
use gpui::TestAppContext;

use crate::support::TestApp;

#[gpui::test]
fn every_sidebar_entry_opens_its_page(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    assert_eq!(app.route(), Route::Overview);
    assert!(app.shown("page overview Overview"));
    for route in [
        Route::Sessions,
        Route::Plugins,
        Route::Deployments,
        Route::Accounts,
        Route::Settings,
        Route::Overview,
    ] {
        app.click(route.nav_selector());
        assert_eq!(app.route(), route, "clicking {:?}", route.nav_selector());
        let title = app.text(route.title_key());
        assert!(
            app.shown(&format!("page {} {title}", route.slug())),
            "the {route:?} page is drawn"
        );
        for other in Route::ALL.into_iter().filter(|other| *other != route) {
            let title = app.text(other.title_key());
            assert!(
                !app.shown(&format!("page {} {title}", other.slug())),
                "{other:?} is not drawn while {route:?} shows"
            );
        }
    }
}

#[gpui::test]
fn signed_out_the_account_slot_leads_to_accounts(cx: &mut TestAppContext) {
    let options =
        crate::support::Options::scenario(colab_playground_core::mock::MockScenario::signed_out());
    let mut app = TestApp::with(cx, options);
    assert!(app.shown(sidebar::USAGE_SLOT_SELECTOR));
    assert!(
        app.shown(sidebar::DEMO_BADGE_SELECTOR),
        "mock mode says it is demo data"
    );
    app.click(sidebar::ACCOUNT_SLOT_SELECTOR);
    assert_eq!(app.route(), Route::Accounts);
}

#[gpui::test]
fn every_page_renders_in_every_language(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    for pref in colab_playground::settings::LanguagePref::ALL {
        app.update(|state, cx| state.set_language(pref, cx));
        for route in Route::ALL {
            app.update(|state, cx| state.navigate(route, cx));
            let title = app.text(route.title_key());
            assert!(app.shown(&format!("page {} {title}", route.slug())));
        }
    }
}
