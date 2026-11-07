use anyhow::{Context, Result};
use icu_locale_core::Locale;

/// A language the pages speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// Chinese in simplified characters, as mainland China writes it.
    Hans,
    /// Chinese in traditional characters, as Taiwan writes it.
    Hant,
    En,
}

/// One text written in each language, in the order of [`Lang::ALL`].
pub type Text = [&'static str; 3];

impl Lang {
    /// The languages, in the order the language page lists them.
    pub const ALL: [Lang; 3] = [Lang::Hans, Lang::Hant, Lang::En];

    /// The language's name, written in it.
    pub fn name(self) -> &'static str {
        self.pick(["简体中文", "繁體中文", "English"])
    }

    /// The language as a locale, which [`Lang::for_locale`] reads back.
    pub fn tag(self) -> &'static str {
        self.pick(["zh-Hans", "zh-Hant", "en"])
    }

    /// Of the same thing written in each language, in the order of
    /// [`Lang::ALL`], the one in this language.
    pub fn pick<T>(self, [hans, hant, en]: [T; 3]) -> T {
        match self {
            Lang::Hans => hans,
            Lang::Hant => hant,
            Lang::En => en,
        }
    }

    /// The language for a locale: the `zh-CN` a system is built to start
    /// in, or the one it hands the recovery with the work it asks for,
    /// which older systems write `zh_CN`. Chinese in that script or, with
    /// none named, in the one Taiwan, Hong Kong and Macau write in
    /// traditional characters, any other Chinese in simplified ones, and
    /// every other language in English.
    pub fn for_locale(tag: &str) -> Result<Lang> {
        // ICU takes a hyphen alone between the parts of a locale.
        let locale = Locale::try_from_str(&tag.replace('_', "-")).with_context(|| format!("read the locale {tag:?}"))?;
        let id = locale.id;
        if id.language.as_str() != "zh" {
            return Ok(Lang::En);
        }
        let traditional = match id.script {
            Some(script) => script.as_str() == "Hant",
            None => id.region.is_some_and(|r| matches!(r.as_str(), "TW" | "HK" | "MO")),
        };
        Ok(if traditional { Lang::Hant } else { Lang::Hans })
    }
}

/// A text written in each language, made as [`format!`] makes it of the
/// values it names:
/// `tr!(l, "挂载 {base} 失败", "掛載 {base} 失敗", "Couldn't mount {base}")`.
#[macro_export]
macro_rules! tr {
    ($l:expr, $hans:literal, $hant:literal, $en:literal $(,)?) => {
        match $l {
            $crate::lang::Lang::Hans => format!($hans),
            $crate::lang::Lang::Hant => format!($hant),
            $crate::lang::Lang::En => format!($en),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::{TokenStream, TokenTree};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;
    use syn::visit::Visit;

    /// The texts a source file writes: its string literals, those among a
    /// macro's arguments too.
    #[derive(Default)]
    struct Texts(Vec<String>);

    impl Texts {
        fn tokens(&mut self, tokens: TokenStream) {
            for tree in tokens {
                match tree {
                    TokenTree::Literal(lit) => {
                        if let syn::Lit::Str(s) = syn::Lit::new(lit) {
                            self.0.push(s.value());
                        }
                    }
                    TokenTree::Group(group) => self.tokens(group.stream()),
                    TokenTree::Ident(_) | TokenTree::Punct(_) => {}
                }
            }
        }
    }

    impl<'ast> Visit<'ast> for Texts {
        fn visit_lit_str(&mut self, lit: &'ast syn::LitStr) {
            self.0.push(lit.value());
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            self.tokens(mac.tokens.clone());
        }
    }

    #[test]
    fn the_typeface_draws_every_text_the_pages_write() {
        let f = crate::font::bundled();
        let mut texts = Texts::default();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in fs::read_dir(&src).unwrap() {
            let path = entry.unwrap().path();
            let file = syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            texts.visit_file(&file);
        }
        assert!(texts.0.iter().any(|t| t.contains("簽章")), "the texts in traditional characters are read");
        let missing: BTreeSet<char> = texts.0.iter().flat_map(|t| t.chars()).filter(|&ch| !ch.is_control() && !f.body.has(ch)).collect();
        assert!(missing.is_empty(), "MiSans draws none of {missing:?}");
    }

    #[test]
    fn a_locale_picks_the_language_and_its_characters() {
        let of = |tag| Lang::for_locale(tag).unwrap();
        assert_eq!(of("zh-CN"), Lang::Hans, "a China system, as the 17 Ultra's");
        assert_eq!(of("zh_CN"), Lang::Hans, "as older systems write it");
        assert_eq!(of("zh-SG"), Lang::Hans);
        assert_eq!(of("zh"), Lang::Hans);
        assert_eq!(of("zh-TW"), Lang::Hant);
        assert_eq!(of("zh-HK"), Lang::Hant);
        assert_eq!(of("zh-Hant-TW"), Lang::Hant);
        assert_eq!(of("zh-Hans-HK"), Lang::Hans, "a script named wins over the region");
        assert_eq!(of("en-US"), Lang::En);
        assert_eq!(of("ja-JP"), Lang::En, "a language the pages do not speak");
        assert!(Lang::for_locale("not a locale").is_err());
    }

    #[test]
    fn each_language_s_locale_reads_back_as_it() {
        for lang in Lang::ALL {
            assert_eq!(Lang::for_locale(lang.tag()).unwrap(), lang);
        }
    }

    #[test]
    fn a_text_is_picked_and_made_in_the_language() {
        assert_eq!(Lang::Hant.pick(["设置", "設定", "Settings"]), "設定");
        let base = "vendor";
        assert_eq!(tr!(Lang::En, "挂载 {base} 失败", "掛載 {base} 失敗", "Couldn't mount {base}"), "Couldn't mount vendor");
        assert_eq!(Lang::ALL.map(Lang::name), ["简体中文", "繁體中文", "English"]);
    }
}
