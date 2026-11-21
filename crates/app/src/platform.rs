//! Everything that differs per operating system lives here, behind `cfg`; no other module of
//! this crate's `src/` names a target OS.
//!
//! Ely bundles Latin fonts only (Inter, JetBrains Mono, IBM Plex Sans). Han characters, kana
//! and hangul render through the platform's font fallback; the app sets an explicit fallback list
//! on its root text style that leads with the shown language's own faces. Han characters are
//! shared, and the first listed face that has one decides its glyph shapes, so Simplified text
//! uses an SC face, Traditional text a TC face, Japanese a Japanese face and Korean a Korean face
//! (system fallback alone picks by OS locale and can draw one language with another's glyph
//! shapes). The other languages' faces follow, because the language select names every language
//! in its own script, so kana and hangul still find a face whatever language is shown. Only
//! families that are installed are listed: DirectWrite skips a missing family, and gpui's Linux
//! loader is meant to drop one too, but it propagates loader errors with `?`, so an unknown family
//! may break font loading there. Text drawn outside the root view (Ely tooltips set their own
//! font family) gets no list and relies on the platform's fallback alone.

use ely_gpui_component::theme::{ActiveTheme, Platform};
use gpui::{
    App, Decorations, FontFallbacks, Pixels, Point, WindowBackgroundAppearance, WindowDecorations,
};

use crate::i18n::Language;

/// Who draws which part of the window frame. The app never shows the OS title bar; this says
/// what replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chrome {
    /// Windows: the app draws themed caption buttons; dragging, double-click to maximise, Aero
    /// Snap, the snap-layout flyout and edge resizing stay native through gpui's hit-test areas.
    Windows,
    /// macOS: the system draws its traffic lights over the transparent title bar, and the strip
    /// holds nothing else.
    Mac,
    /// Linux whose compositor wants client-side decorations: the app draws buttons and resize
    /// borders.
    LinuxClient,
    /// Linux whose compositor draws the frame (server-side decorations): left alone.
    LinuxServer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitleBarLayout {
    /// Ely window controls drawn at the strip's end, in this platform's manner.
    pub controls: Option<Platform>,
    /// The app mark and name at the strip's start. Not on macOS: the traffic lights sit there
    /// and the menu bar names the app, as in other Mac apps.
    pub mark: bool,
    /// Whether the strip moves the window (and maximises on double-click).
    pub draggable: bool,
}

impl Chrome {
    /// Every variant, for tests that check each layout on any host.
    pub const ALL: [Chrome; 4] = [
        Chrome::Windows,
        Chrome::Mac,
        Chrome::LinuxClient,
        Chrome::LinuxServer,
    ];

    pub fn layout(self) -> TitleBarLayout {
        match self {
            Chrome::Windows => TitleBarLayout {
                controls: Some(Platform::Windows),
                mark: true,
                draggable: true,
            },
            Chrome::Mac => TitleBarLayout {
                controls: None,
                mark: false,
                draggable: true,
            },
            Chrome::LinuxClient => TitleBarLayout {
                controls: Some(Platform::Linux),
                mark: true,
                draggable: true,
            },
            Chrome::LinuxServer => TitleBarLayout {
                controls: None,
                mark: true,
                draggable: false,
            },
        }
    }
}

/// This OS's chrome, given the decorations the window ended up with
/// (`window.window_decorations()`; only Linux compositors choose).
pub fn chrome(decorations: Decorations) -> Chrome {
    if cfg!(target_os = "windows") {
        Chrome::Windows
    } else if cfg!(target_os = "macos") {
        Chrome::Mac
    } else {
        match decorations {
            Decorations::Client { .. } => Chrome::LinuxClient,
            Decorations::Server => Chrome::LinuxServer,
        }
    }
}

/// Decorations to ask for: client-side on Linux (the compositor may still insist on its own),
/// the platform's default elsewhere.
pub fn window_decorations() -> Option<WindowDecorations> {
    cfg!(any(target_os = "linux", target_os = "freebsd")).then_some(WindowDecorations::Client)
}

/// A see-through window on Linux, so the client-side resize margin and shadow show the
/// desktop; opaque elsewhere.
pub fn window_background() -> WindowBackgroundAppearance {
    if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

/// Where macOS draws the traffic lights: centred in Ely's title-bar height (ADR 0036). AppKit
/// answers on the main thread only, so tests get the system default.
pub fn traffic_light_position(cx: &App) -> Option<Point<Pixels>> {
    (cfg!(target_os = "macos") && !cx.is_test()).then(|| cx.theme().traffic_light_origin())
}

/// Preferred CJK families on this OS, best first.
struct CjkFonts {
    simplified: &'static [&'static str],
    traditional: &'static [&'static str],
    japanese: &'static [&'static str],
    korean: &'static [&'static str],
}

/// gpui lists a Windows family by its name in the user's locale where the font has one. The `UI`
/// faces have the same name in every locale; Malgun Gothic has no `UI` face, so it is listed
/// under both of its names (a Korean Windows reports 맑은 고딕).
#[cfg(target_os = "windows")]
const CJK_FONTS: CjkFonts = CjkFonts {
    simplified: &["Microsoft YaHei UI", "Microsoft YaHei"],
    traditional: &["Microsoft JhengHei UI", "Microsoft JhengHei"],
    japanese: &["Yu Gothic UI", "Yu Gothic", "Meiryo UI", "Meiryo"],
    korean: &["Malgun Gothic", "맑은 고딕"],
};

#[cfg(target_os = "macos")]
const CJK_FONTS: CjkFonts = CjkFonts {
    simplified: &["PingFang SC", "Hiragino Sans GB", "Heiti SC"],
    traditional: &["PingFang TC", "Heiti TC"],
    japanese: &["Hiragino Sans", "Hiragino Kaku Gothic ProN"],
    korean: &["Apple SD Gothic Neo"],
};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const CJK_FONTS: CjkFonts = CjkFonts {
    simplified: &[
        "Noto Sans CJK SC",
        "Source Han Sans SC",
        "Noto Sans SC",
        "WenQuanYi Micro Hei",
    ],
    traditional: &["Noto Sans CJK TC", "Source Han Sans TC", "Noto Sans TC"],
    japanese: &["Noto Sans CJK JP", "Source Han Sans JP", "Noto Sans JP"],
    korean: &["Noto Sans CJK KR", "Source Han Sans KR", "Noto Sans KR"],
};

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd"
)))]
const CJK_FONTS: CjkFonts = CjkFonts {
    simplified: &[],
    traditional: &[],
    japanese: &[],
    korean: &[],
};

/// What the first checklist step checks: WSL on Windows (the CLI runs inside it), the host's
/// shell tools elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentKind {
    /// WSL with a Linux distribution.
    Wsl,
    /// `bash` and `ssh` on the host.
    ShellTools,
}

pub fn environment_kind() -> EnvironmentKind {
    if cfg!(windows) {
        EnvironmentKind::Wsl
    } else {
        EnvironmentKind::ShellTools
    }
}

/// Whether the CLI runs inside WSL here (Settings then offers the distribution).
pub fn uses_wsl() -> bool {
    environment_kind() == EnvironmentKind::Wsl
}

/// Microsoft's guide to installing WSL, offered when it is missing.
pub const WSL_INSTALL_URL: &str = "https://learn.microsoft.com/windows/wsl/install";

/// The OS's preferred locale, such as `zh-Hant-TW`, `zh-CN` or `en_US.UTF-8`.
pub fn os_locale() -> Option<String> {
    sys_locale::get_locale()
}

/// Whether the system that started the app added `arg`: old macOS launchers append a process
/// serial number (`-psn_…`), which is not ours to judge.
pub fn is_launcher_argument(arg: &str) -> bool {
    cfg!(target_os = "macos") && arg.starts_with("-psn_")
}

/// The operating system and processor this build is for, as Rust names them (`windows,
/// x86_64`), for a bug report.
pub fn system() -> String {
    format!("{}, {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// CJK families to try for `language`, best first: Simplified, Traditional, Japanese, Korean.
/// Both languages get every family, since user data (session names, model ids, plugin labels)
/// can be in any of them.
pub fn cjk_font_candidates(language: Language) -> Vec<&'static str> {
    let CjkFonts {
        simplified,
        traditional,
        japanese,
        korean,
    } = CJK_FONTS;
    let order = match language {
        Language::EnUs | Language::ZhCn => [simplified, traditional, japanese, korean],
    };
    order.into_iter().flatten().copied().collect()
}

/// The fallback list for the root text style: the candidates for `language` that appear in
/// `installed` (the text system's `all_font_names()`), or `None` when none do, which leaves the
/// platform's own fallback in charge.
pub fn font_fallbacks(language: Language, installed: &[String]) -> Option<FontFallbacks> {
    let chosen: Vec<String> = cjk_font_candidates(language)
        .into_iter()
        .filter(|family| installed.iter().any(|name| name == family))
        .map(str::to_owned)
        .collect();
    if chosen.is_empty() {
        log::warn!(
            "fonts: none of {:?} is installed; CJK text relies on the platform's fallback",
            cjk_font_candidates(language)
        );
        return None;
    }
    log::info!("fonts: CJK fallbacks for {}: {chosen:?}", language.tag());
    Some(FontFallbacks::from_fonts(chosen))
}

#[cfg(test)]
mod tests {
    use ely_gpui_component::theme::Platform;

    use super::{CJK_FONTS, Chrome, cjk_font_candidates, font_fallbacks};
    use crate::i18n::Language;

    #[test]
    fn each_chrome_draws_only_the_controls_the_system_does_not() {
        let windows = Chrome::Windows.layout();
        assert_eq!(windows.controls, Some(Platform::Windows));
        assert!(windows.draggable && windows.mark);

        let mac = Chrome::Mac.layout();
        assert_eq!(mac.controls, None, "macOS keeps the system traffic lights");
        assert!(mac.draggable);
        assert!(!mac.mark, "and nothing beside them");

        let client = Chrome::LinuxClient.layout();
        assert_eq!(client.controls, Some(Platform::Linux));
        assert!(client.draggable && client.mark);

        let server = Chrome::LinuxServer.layout();
        assert_eq!(
            server.controls, None,
            "the compositor draws its own buttons"
        );
        assert!(!server.draggable && server.mark);
    }

    #[test]
    fn simplified_faces_lead_for_both_languages() {
        for language in Language::ALL {
            let candidates = cjk_font_candidates(language);
            assert_eq!(
                &candidates[..CJK_FONTS.simplified.len()],
                CJK_FONTS.simplified,
                "{}",
                language.tag()
            );
        }
        let cn = cjk_font_candidates(Language::ZhCn);
        assert_eq!(cjk_font_candidates(Language::EnUs), cn);
        assert_eq!(
            cn[CJK_FONTS.simplified.len()..][..CJK_FONTS.traditional.len()],
            *CJK_FONTS.traditional,
            "Traditional faces come next for Simplified Chinese and English"
        );
    }

    #[test]
    fn every_language_lists_every_family_once() {
        let mut all: Vec<&str> = [
            CJK_FONTS.simplified,
            CJK_FONTS.traditional,
            CJK_FONTS.japanese,
            CJK_FONTS.korean,
        ]
        .concat();
        all.sort_unstable();
        let unique = all.len();
        all.dedup();
        assert_eq!(all.len(), unique, "a family is listed twice");
        for language in Language::ALL {
            let mut candidates = cjk_font_candidates(language);
            candidates.sort_unstable();
            assert_eq!(
                candidates,
                all,
                "{} misses a face, so some script could go without one",
                language.tag()
            );
        }
    }

    #[test]
    fn only_installed_families_are_listed() {
        assert!(font_fallbacks(Language::ZhCn, &[]).is_none());
        let candidates = cjk_font_candidates(Language::EnUs);
        let Some(last) = candidates.last() else {
            return;
        };
        let installed = vec!["Inter".to_owned(), (*last).to_owned()];
        let fallbacks = font_fallbacks(Language::EnUs, &installed).expect("one is installed");
        assert_eq!(fallbacks.fallback_list(), [(*last).to_owned()]);
    }
}

gpui::actions!(colab_playground, [Quit]);

/// The app menu: the app's name with Quit. Built on every OS (so tests check it), installed
/// only on macOS by [`install_quit`].
pub fn app_menus(cx: &App) -> Vec<gpui::Menu> {
    vec![gpui::Menu {
        name: crate::i18n::tr(cx, crate::i18n::keys::APP_NAME),
        items: vec![gpui::MenuItem::action(
            crate::i18n::tr(cx, crate::i18n::keys::MENU_QUIT),
            Quit,
        )],
        disabled: false,
    }]
}

/// Cmd+Q for [`Quit`] (installed on macOS only; built everywhere so tests check it).
pub fn quit_bindings() -> Vec<gpui::KeyBinding> {
    vec![gpui::KeyBinding::new("cmd-q", Quit, None)]
}

/// macOS has no Cmd+Q without an app menu (gpui builds the menu bar only from `set_menus`),
/// so bind it and install the minimal menu there. Windows and Linux quit by closing the
/// window, which the close check already covers.
pub fn install_quit(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        cx.bind_keys(quit_bindings());
        let menus = app_menus(cx);
        cx.set_menus(menus);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}
