use std::collections::BTreeSet;

use super::{CATALOGS, Language, catalog, keys, resolve};
use crate::settings::LanguagePref;

fn arguments(message: &str) -> BTreeSet<&str> {
    message
        .split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
        .collect()
}

#[test]
fn every_catalog_holds_exactly_the_declared_keys_once() {
    let declared: BTreeSet<&str> = keys::ALL.iter().copied().collect();
    assert_eq!(declared.len(), keys::ALL.len(), "a key is declared twice");
    for key in &declared {
        assert!(
            key.starts_with("app."),
            "{key} could shadow one of Ely's keys"
        );
    }
    for (tag, messages) in CATALOGS {
        let found: BTreeSet<&str> = messages.iter().map(|(key, _)| *key).collect();
        assert_eq!(found.len(), messages.len(), "{tag} repeats a key");
        let missing: Vec<_> = declared.difference(&found).collect();
        let extra: Vec<_> = found.difference(&declared).collect();
        assert!(missing.is_empty(), "{tag} misses {missing:?}");
        assert!(extra.is_empty(), "{tag} has undeclared {extra:?}");
    }
}

#[test]
fn every_key_resolves_in_every_language_with_the_same_arguments() {
    for language in Language::ALL {
        let i18n = catalog(language);
        assert_eq!(i18n.locale().tag, language.tag());
        for key in keys::ALL {
            let english = CATALOGS[0]
                .1
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, message)| *message)
                .expect("declared keys are in en-US");
            let names = arguments(english);
            let args: Vec<(&str, &str)> = names.iter().map(|name| (*name, "x")).collect();
            let text = i18n.text(key, &args);
            assert!(
                !text.trim().is_empty(),
                "{key} is empty in {}",
                language.tag()
            );
        }
    }
    for (tag, messages) in CATALOGS {
        for (key, message) in messages {
            let english = CATALOGS[0]
                .1
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, message)| *message)
                .expect("same keys");
            assert_eq!(
                arguments(message),
                arguments(english),
                "{key} in {tag} takes other arguments than in en-US"
            );
        }
    }
}

#[test]
fn app_code_names_keys_only_through_the_constants() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // The key declarations, and this test, which has to spell the pattern it looks for.
    let allowed = [
        src.join("i18n").join("mod.rs"),
        src.join("i18n").join("tests.rs"),
    ];
    let mut offenders = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src is readable") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") || allowed.contains(&path) {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            if text.contains("\"app.") {
                offenders.push(path.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these files spell catalog keys as literals instead of i18n::keys constants: {offenders:?}"
    );
}

#[test]
fn os_locales_resolve_to_the_shipped_languages() {
    let cases = [
        ("zh-CN", Language::ZhCn),
        ("zh_CN.UTF-8", Language::ZhCn),
        ("zh", Language::ZhCn),
        ("zh-SG", Language::ZhCn),
        ("zh-Hans-CN", Language::ZhCn),
        ("zh-TW", Language::ZhCn),
        ("zh_HK.UTF-8", Language::ZhCn),
        ("ZH-hant-cn", Language::ZhCn),
        ("ja-JP", Language::EnUs),
        ("ko_KR.UTF-8", Language::EnUs),
        ("en-US", Language::EnUs),
        ("en-GB", Language::EnUs),
        ("zha-CN", Language::EnUs),
        ("C", Language::EnUs),
        ("POSIX", Language::EnUs),
        ("", Language::EnUs),
    ];
    for (locale, expected) in cases {
        assert_eq!(Language::from_os_locale(locale), expected, "{locale:?}");
    }
}

#[test]
fn a_preference_resolves_with_the_os_locale_only_for_system() {
    assert_eq!(resolve(LanguagePref::System, Some("zh-HK")), Language::ZhCn);
    assert_eq!(resolve(LanguagePref::System, Some("ja-JP")), Language::EnUs);
    assert_eq!(resolve(LanguagePref::System, None), Language::EnUs);
    assert_eq!(resolve(LanguagePref::EnUs, Some("zh-CN")), Language::EnUs);
    assert_eq!(resolve(LanguagePref::ZhCn, Some("en-US")), Language::ZhCn);
}

#[test]
fn english_is_the_default_preference() {
    assert_eq!(LanguagePref::default(), LanguagePref::EnUs);
}
