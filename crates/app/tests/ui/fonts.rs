//! CJK rendering through the platform's real text system (not gpui's no-op test one).
//!
//! For each display language it shapes words from that language's UI with Ely's Inter as the
//! primary family and the app's fallback list, and checks that every character gets a real glyph
//! and that the CJK characters (Han, kana, hangul) come from the first fallback face, not from
//! Inter or another language's face. It shapes the language select's names, which every language
//! shows, and checks that they all get real glyphs. It also checks that text set in Inter with no
//! fallback list (Ely overlays such as tooltips, which are not under the app's root text style)
//! still gets real CJK glyphs from the platform's own fallback. It reads the host's fonts, so it
//! runs by default only on Windows, whose standard fonts include Microsoft YaHei UI, Microsoft
//! JhengHei UI, Yu Gothic UI and Malgun Gothic (this `cfg_attr` is the crate's one OS gate outside
//! `platform`). On Linux, run it with `--ignored` where Noto Sans CJK is installed. Leave it
//! ignored on macOS, where creating gpui's windowed platform off the main thread may abort.
//! gpui's headless Windows platform has no text system, so this creates the windowed platform (no
//! window is opened).

use std::sync::Arc;

use colab_playground::{i18n::Language, platform};
use gpui::{
    AssetSource, Font, FontFallbacks, FontFeatures, FontId, FontRun, FontStyle, FontWeight,
    PlatformTextSystem, px,
};

/// Words from each language's UI; the English UI shows Chinese only in language names.
const SAMPLES: [(Language, &str); 2] = [
    (Language::EnUs, "Language: 简体中文"),
    (Language::ZhCn, "设置 · 会话 · 账号 · 简体中文"),
];

/// The language select's choices, each in its own script, shown under every language.
const LANGUAGE_NAMES: &str = "English / 简体中文";

fn font(family: &str, fallbacks: Option<FontFallbacks>) -> Font {
    Font {
        family: family.to_owned().into(),
        features: FontFeatures::default(),
        fallbacks,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// Han characters, kana and hangul.
fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
        || ('\u{3040}'..='\u{30ff}').contains(&ch)
        || ('\u{ac00}'..='\u{d7af}').contains(&ch)
}

/// Shapes `sample` in `primary`; checks every visible character has a real glyph and returns
/// the fonts its CJK characters came from.
fn cjk_fonts(
    text_system: &Arc<dyn PlatformTextSystem>,
    sample: &str,
    primary: FontId,
) -> Vec<FontId> {
    let layout = text_system.layout_line(
        sample,
        px(14.0),
        &[FontRun {
            len: sample.len(),
            font_id: primary,
        }],
    );
    let mut fonts = Vec::new();
    for run in &layout.runs {
        for glyph in &run.glyphs {
            let ch = sample[glyph.index..].chars().next().expect("a character");
            if ch.is_whitespace() {
                continue;
            }
            assert_ne!(glyph.id.0, 0, "{ch:?} in {sample:?} has no glyph (tofu)");
            if is_cjk(ch) {
                fonts.push(run.font_id);
            }
        }
    }
    assert!(
        fonts.len() >= 4,
        "{sample:?} was shaped with {} CJK glyphs",
        fonts.len()
    );
    fonts
}

#[test]
#[cfg_attr(
    not(target_os = "windows"),
    ignore = "reads the host's fonts; run with --ignored where a CJK font is installed"
)]
fn cjk_characters_get_real_glyphs_from_the_fallback_faces() {
    let text_system = gpui_platform::current_platform(false).text_system();
    let inter = ely_gpui_component::Assets
        .load("fonts/Inter-Regular.ttf")
        .expect("Ely's assets load")
        .expect("Inter is bundled");
    text_system.add_fonts(vec![inter]).expect("Inter registers");
    let installed = text_system.all_font_names();
    assert!(installed.iter().any(|name| name == "Inter"));
    let bare = text_system
        .font_id(&font("Inter", None))
        .expect("Inter resolves");
    for (language, sample) in SAMPLES {
        let fallbacks = platform::font_fallbacks(language, &installed).unwrap_or_else(|| {
            panic!(
                "none of {:?} is installed",
                platform::cjk_font_candidates(language)
            )
        });
        let first = fallbacks.fallback_list()[0].clone();
        let face = text_system
            .font_id(&font(&first, None))
            .expect("the fallback face resolves");

        let inter = text_system
            .font_id(&font("Inter", Some(fallbacks.clone())))
            .expect("Inter resolves");
        for used in cjk_fonts(&text_system, sample, inter) {
            assert_eq!(
                used,
                face,
                "{sample:?} ({}) should use {first}",
                language.tag()
            );
        }
        for used in cjk_fonts(&text_system, LANGUAGE_NAMES, inter) {
            assert_ne!(used, bare, "Inter cannot draw CJK characters");
        }

        for text in [sample, LANGUAGE_NAMES] {
            for used in cjk_fonts(&text_system, text, bare) {
                assert_ne!(used, bare, "Inter cannot draw CJK characters");
            }
        }
        eprintln!(
            "{}: CJK glyphs from {first}; fallbacks {:?}",
            language.tag(),
            fallbacks.fallback_list()
        );
    }
}
