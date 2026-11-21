use colab_playground::{
    i18n::{Language, keys},
    route::Route,
    settings::{AppearancePref, LanguagePref},
    ui::{
        metrics::WINDOW_MIN_SIZE,
        pages::settings::{
            APPEARANCE_SELECTOR, FILE_PATH_SELECTOR, FILE_SELECTOR, LANGUAGE_SELECTOR,
            OPEN_FOLDER_SELECTOR,
        },
        root::PAGE_SELECTOR,
    },
};
use ely_gpui_component::theme::Mode;
use gpui::TestAppContext;

use crate::support::{ConfigDir, Options, TestApp};

const LANGUAGES: usize = LanguagePref::ALL.len();

fn index(pref: LanguagePref) -> usize {
    LanguagePref::ALL
        .iter()
        .position(|each| *each == pref)
        .expect("offered")
}

fn open_settings(cx: &mut TestAppContext, options: Options) -> TestApp<'_> {
    let mut app = TestApp::with(cx, options);
    app.click(Route::Settings.nav_selector());
    assert_eq!(app.route(), Route::Settings);
    app
}

#[gpui::test]
fn picking_a_language_redraws_in_it_and_saves_it(cx: &mut TestAppContext) {
    let mut app = open_settings(cx, Options::default());
    assert_eq!(app.language(), Language::EnUs);
    assert!(app.shown("page settings Settings"));

    app.pick_option(LANGUAGE_SELECTOR, index(LanguagePref::ZhCn), LANGUAGES);
    assert_eq!(app.language(), Language::ZhCn);
    assert_eq!(
        app.read(|state, _| state.settings().language),
        LanguagePref::ZhCn
    );
    assert!(
        app.shown("page settings 设置"),
        "the page title is redrawn in Simplified Chinese"
    );
    assert!(!app.shown("page settings Settings"));
    assert_eq!(app.text(keys::NAV_SESSIONS), "会话");
    assert_eq!(
        app.saved_settings().map(|settings| settings.language),
        Some(LanguagePref::ZhCn)
    );

    app.pick_option(LANGUAGE_SELECTOR, index(LanguagePref::EnUs), LANGUAGES);
    assert_eq!(app.language(), Language::EnUs);
    assert!(app.shown("page settings Settings"));
    assert_eq!(
        app.saved_settings().map(|settings| settings.language),
        Some(LanguagePref::EnUs)
    );
}

#[gpui::test]
fn english_is_the_default_whatever_the_os_speaks(cx: &mut TestAppContext) {
    let options = Options {
        os_locale: Some("zh-CN".into()),
        ..Options::default()
    };
    let mut app = open_settings(cx, options);
    assert_eq!(
        app.read(|state, _| state.settings().language),
        LanguagePref::EnUs
    );
    assert_eq!(app.language(), Language::EnUs);
    assert!(app.shown("page settings Settings"));
}

#[gpui::test]
fn system_language_follows_the_os_locale(cx: &mut TestAppContext) {
    let options = Options {
        os_locale: Some("zh-HK".into()),
        ..Options::default()
    };
    let mut app = open_settings(cx, options);
    app.pick_option(LANGUAGE_SELECTOR, index(LanguagePref::System), LANGUAGES);
    assert_eq!(
        app.language(),
        Language::ZhCn,
        "zh-HK shows Simplified Chinese, the only Chinese the app ships"
    );
    assert!(app.shown("page settings 设置"));

    app.pick_option(LANGUAGE_SELECTOR, index(LanguagePref::EnUs), LANGUAGES);
    assert_eq!(app.language(), Language::EnUs);
    app.pick_option(LANGUAGE_SELECTOR, index(LanguagePref::System), LANGUAGES);
    assert_eq!(app.language(), Language::ZhCn);
    assert_eq!(
        app.saved_settings().map(|settings| settings.language),
        Some(LanguagePref::System)
    );
}

#[gpui::test]
fn ely_component_strings_follow_the_app_language(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    let ely = |app: &mut TestApp<'_>, key: &str| {
        app.cx
            .update(|_, cx| ely_gpui_component::i18n::text(cx, key, &[]))
    };
    assert_eq!(ely(&mut app, "theme.light"), "Light");
    app.update(|state, cx| state.set_language(LanguagePref::ZhCn, cx));
    assert_eq!(ely(&mut app, "theme.light"), "浅色");
}

#[gpui::test]
fn clicking_an_appearance_switches_the_theme_and_saves_it(cx: &mut TestAppContext) {
    let mut app = open_settings(cx, Options::default());
    assert!(app.shown(APPEARANCE_SELECTOR));
    assert_eq!(
        app.mode(),
        Mode::Light,
        "the test platform reports a light system"
    );

    app.click("segment theme.dark");
    assert_eq!(app.mode(), Mode::Dark);
    assert_eq!(
        app.read(|state, _| state.settings().appearance),
        AppearancePref::Dark
    );
    assert_eq!(
        app.saved_settings().map(|settings| settings.appearance),
        Some(AppearancePref::Dark)
    );

    app.click("segment theme.light");
    assert_eq!(app.mode(), Mode::Light);

    app.click("segment theme.dark");
    app.click("segment theme.system");
    assert_eq!(
        app.mode(),
        Mode::Light,
        "System shows the system's light mode"
    );
    assert_eq!(
        app.saved_settings().map(|settings| settings.appearance),
        Some(AppearancePref::System)
    );
}

/// gpui's test window cannot be told that the OS appearance changed (its
/// `simulate_appearance_change` is crate-private), so this drives the handler the root view's
/// `observe_window_appearance` subscription calls.
#[gpui::test]
fn system_appearance_changes_are_followed_only_under_system(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    assert_eq!(app.mode(), Mode::Light);
    app.update(|state, cx| state.system_appearance_changed(Mode::Dark, cx));
    assert_eq!(app.mode(), Mode::Dark, "System follows the OS into dark");
    app.update(|state, cx| state.set_appearance(AppearancePref::Light, cx));
    assert_eq!(app.mode(), Mode::Light);
    app.update(|state, cx| state.system_appearance_changed(Mode::Light, cx));
    app.update(|state, cx| state.system_appearance_changed(Mode::Dark, cx));
    assert_eq!(app.mode(), Mode::Light, "an explicit Light ignores the OS");
    app.update(|state, cx| state.set_appearance(AppearancePref::System, cx));
    assert_eq!(
        app.mode(),
        Mode::Dark,
        "back to System picks up the OS's dark mode"
    );
}

#[gpui::test]
fn saved_settings_apply_at_startup(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith(r#"{ "language": "zh-TW", "appearance": "dark" }"#.into()),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert_eq!(
        app.language(),
        Language::ZhCn,
        "Traditional Chinese, no longer shipped, reads as Simplified"
    );
    assert_eq!(app.mode(), Mode::Dark);
    assert!(app.shown("page overview 概览"));
    assert!(app.toasts().is_empty());
}

#[gpui::test]
fn a_saved_language_no_longer_shipped_falls_back_to_english(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith(r#"{ "language": "ja-JP" }"#.into()),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert_eq!(app.language(), Language::EnUs);
    assert!(app.toasts().is_empty());
}

#[gpui::test]
fn a_corrupt_settings_file_starts_with_defaults_and_says_so(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempWith("{ this is not json".into()),
        ..Options::default()
    };
    let mut app = TestApp::with(cx, options);
    assert_eq!(app.language(), Language::EnUs);
    assert_eq!(app.mode(), Mode::Light);
    let reset = app.text(keys::TOAST_SETTINGS_RESET_TITLE);
    assert_eq!(app.toasts(), [reset]);
    let dir = app.config_dir().expect("a temp dir").to_path_buf();
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json.corrupt")).expect("kept aside"),
        "{ this is not json"
    );
    app.update(|state, cx| state.set_appearance(AppearancePref::Dark, cx));
    assert_eq!(
        app.saved_settings().map(|settings| settings.appearance),
        Some(AppearancePref::Dark),
        "the next change writes a fresh file"
    );
}

#[gpui::test]
fn without_a_config_dir_settings_still_apply(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::None,
        ..Options::default()
    };
    let mut app = open_settings(cx, options);
    assert!(app.read(|state, _| state.store().file().is_none()));
    app.click("segment theme.dark");
    assert_eq!(app.mode(), Mode::Dark);
    assert!(app.toasts().is_empty(), "nothing to save is not a failure");
}

/// A config folder deep enough that its path cannot fit beside the row's title.
fn deep_folder() -> String {
    ["colab-playground-settings-live-here"; 6].join("/")
}

#[gpui::test]
fn the_settings_file_path_fits_the_row_at_the_minimum_window_size(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempNested(deep_folder()),
        ..Options::default()
    };
    let mut app = open_settings(cx, options);
    for window in [crate::support::WINDOW, WINDOW_MIN_SIZE] {
        app.resize(window);
        let page = app.bounds(PAGE_SELECTOR).expect("the page area");
        let control = app
            .bounds(FILE_SELECTOR)
            .expect("the settings file control");
        let path = app.bounds(FILE_PATH_SELECTOR).expect("the path");
        let open = app
            .bounds(OPEN_FOLDER_SELECTOR)
            .expect("the open-folder button");
        assert!(
            control.right() <= page.right(),
            "at {window:?} the control ends at {:?}, past the page's {:?}",
            control.right(),
            page.right()
        );
        assert!(open.right() <= control.right() && open.left() > path.right());
        assert!(
            path.size.width > gpui::px(40.0),
            "some of the path still shows"
        );
    }
}

#[gpui::test]
fn open_folder_creates_and_opens_the_config_folder(cx: &mut TestAppContext) {
    let options = Options {
        config: ConfigDir::TempNested("not-created-yet".into()),
        ..Options::default()
    };
    let mut app = open_settings(cx, options);
    let dir = app.config_dir().expect("a config dir").to_path_buf();
    assert!(!dir.exists());
    app.click(OPEN_FOLDER_SELECTOR);
    assert!(
        dir.is_dir(),
        "the folder is created so there is something to open"
    );
    let opened = app.opened_url().expect("a URL was opened");
    assert!(opened.starts_with("file:///"), "{opened}");
    assert!(opened.ends_with("/not-created-yet/"), "{opened}");
    assert!(app.toasts().is_empty());
}
