use anyhow::{Context, Result};
use icu_casemap::CaseMapper;
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};

// The languages, from the build script's list of them: `Lang`, how many
// there are, their order, and each one's locale and its name for itself.
include!(concat!(env!("OUT_DIR"), "/lang.rs"));

/// One text written in each language, in the order of [`Lang::ALL`].
pub type Text = [&'static str; Lang::COUNT];

/// A name that every language writes the same.
pub const fn same(name: &'static str) -> Text {
    [name; Lang::COUNT]
}

thread_local! {
    /// How each language's numbers choose among the forms a text takes.
    static PLURALS: Vec<PluralRules> = Lang::ALL
        .iter()
        .map(|l| {
            let locale = Locale::try_from_str(l.tag()).expect("a language's tag is a locale");
            PluralRules::try_new_cardinal(locale.into()).expect("the plural rules of every language are built in")
        })
        .collect();
}

impl Lang {
    /// Of the same thing written in each language, in the order of
    /// [`Lang::ALL`], the one in this language.
    pub fn pick<T: Copy>(self, each: [T; Lang::COUNT]) -> T {
        each[self as usize]
    }

    /// Of something written in the two kinds of Chinese characters and in
    /// English alone, as a phone's own description of itself is, the one
    /// this language reads: its own, or else the English.
    pub fn among<T>(self, hans: T, hant: T, en: T) -> T {
        match self {
            Lang::Hans => hans,
            Lang::Hant => hant,
            _ => en,
        }
    }

    /// Which of the forms the language's texts take by number a count of
    /// `n` asks for.
    pub fn plural(self, n: u64) -> PluralCategory {
        PLURALS.with(|rules| rules[self as usize].category_for(n))
    }

    /// The language as the locale its rules are looked up by.
    fn locale(self) -> Locale {
        Locale::try_from_str(self.tag()).expect("a language's tag is a locale")
    }

    /// `text` in small letters, as the language makes them small.
    pub fn lowercase(self, text: &str) -> String {
        String::from(CaseMapper::new().lowercase_to_string(text, &self.locale().id))
    }

    /// The language for a locale: the `zh-CN` a system is built to start
    /// in, or the one it hands the recovery with the work it asks for,
    /// which older systems write `zh_CN`. Chinese in that script or, with
    /// none named, in the one Taiwan, Hong Kong and Macau write in
    /// traditional characters, any other Chinese in simplified ones; a
    /// language the pages speak wherever it is spoken, as Brazil's
    /// Portuguese is read in Portugal too; and every other language in
    /// English.
    pub fn for_locale(tag: &str) -> Result<Lang> {
        // ICU takes a hyphen alone between the parts of a locale.
        let locale = Locale::try_from_str(&tag.replace('_', "-")).with_context(|| format!("read the locale {tag:?}"))?;
        let id = locale.id;
        // Android wrote Indonesian by the code ISO 639 had retired, and
        // older systems still do.
        let language = match id.language.as_str() {
            "in" => "id",
            other => other,
        };
        if language == "zh" {
            let traditional = match id.script {
                Some(script) => script.as_str() == "Hant",
                None => id.region.is_some_and(|r| matches!(r.as_str(), "TW" | "HK" | "MO")),
            };
            return Ok(if traditional { Lang::Hant } else { Lang::Hans });
        }
        Ok(Lang::ALL.into_iter().find(|l| l.locale().id.language.as_str() == language).unwrap_or(Lang::En))
    }
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

    /// Read into `texts` what every source file in `folder` writes, those
    /// in the folders under it too.
    fn read(texts: &mut Texts, folder: &Path) {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                read(texts, &path);
                continue;
            }
            let file = syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            texts.visit_file(&file);
        }
    }

    /// Every text a table of a language's file holds, those of the tables
    /// under it too.
    fn written(table: &toml::Table, texts: &mut Vec<String>) {
        for value in table.values() {
            match value {
                toml::Value::String(text) => texts.push(text.clone()),
                toml::Value::Table(inner) => written(inner, texts),
                _ => {}
            }
        }
    }

    #[test]
    fn the_typeface_draws_every_text_the_pages_write() {
        let f = crate::font::bundled();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut source = Texts::default();
        read(&mut source, &manifest.join("src"));
        assert!(source.0.iter().any(|t| t.contains("Fastboot")), "the texts the source writes itself are read");
        let unwritten = |texts: &[String]| texts.iter().flat_map(|t| t.chars()).filter(|&ch| !ch.is_control() && !f.body.has(ch)).collect::<BTreeSet<char>>();
        assert!(unwritten(&source.0).is_empty(), "MiSans draws none of {:?}, which the source writes", unwritten(&source.0));
        for lang in Lang::ALL {
            let file = manifest.join("locales").join(format!("{}.toml", lang.tag()));
            let table: toml::Table = fs::read_to_string(&file).unwrap().parse().unwrap();
            let mut texts = Vec::new();
            written(&table, &mut texts);
            assert!(texts.len() > 1000, "the texts of {lang:?} are read");
            assert!(unwritten(&texts).is_empty(), "MiSans draws none of {:?}, which {lang:?} writes", unwritten(&texts));
        }
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
        assert_eq!(of("tr-TR"), Lang::Tr);
        assert_eq!(of("ru_RU"), Lang::Ru);
        assert_eq!(of("uk-UA"), Lang::Uk);
        assert_eq!([of("es-ES"), of("es-MX"), of("es-419")], [Lang::Es; 3], "Spanish wherever it is spoken");
        assert_eq!([of("pt-BR"), of("pt-PT")], [Lang::PtBr; 2], "Brazil's Portuguese is read in Portugal too");
        assert_eq!([of("id-ID"), of("in-ID"), of("in_ID")], [Lang::Id; 3], "by the code Android wrote Indonesian with, too");
        assert_eq!(of("vi-VN"), Lang::Vi);
        assert_eq!([of("de-DE"), of("de-AT")], [Lang::De; 2]);
        assert_eq!([of("fr-FR"), of("fr-CA")], [Lang::Fr; 2]);
        assert_eq!(of("it-IT"), Lang::It);
        assert_eq!(of("pl-PL"), Lang::Pl);
        assert_eq!(of("ja-JP"), Lang::Ja);
        assert_eq!(of("ko-KR"), Lang::En, "a language the pages do not speak");
        assert_eq!(of("ar-EG"), Lang::En);
        assert!(Lang::for_locale("not a locale").is_err());
    }

    #[test]
    fn each_language_s_locale_reads_back_as_it() {
        for lang in Lang::ALL {
            assert_eq!(Lang::for_locale(lang.tag()).unwrap(), lang);
        }
    }

    #[test]
    fn each_language_names_itself_and_no_other() {
        let names: BTreeSet<&str> = Lang::ALL.map(Lang::name).into_iter().collect();
        assert_eq!(names.len(), Lang::COUNT);
        assert_eq!(Lang::ALL.map(Lang::name)[..4], ["简体中文", "繁體中文", "English", "Türkçe"]);
        assert_eq!(Lang::Ja.name(), "日本語");
    }

    #[test]
    fn a_text_is_picked_and_made_in_the_language() {
        use crate::texts;
        assert_eq!(Lang::Hant.pick(texts::page::SETTINGS), "設定");
        assert_eq!(texts::mount::failed(Lang::En, "vendor", "busy"), "Couldn't mount vendor: busy");
        assert_eq!(texts::mount::failed(Lang::Hans, "vendor", "busy"), "挂载 vendor 失败：busy");
        assert_eq!(same("EDL"), [Lang::En; Lang::COUNT].map(|_| "EDL"));
        assert_eq!(Lang::En.among("骁龙", "驍龍", "Snapdragon"), "Snapdragon");
        assert_eq!(Lang::Tr.among("骁龙", "驍龍", "Snapdragon"), "Snapdragon", "a phone describes itself in Chinese and English alone");
        assert_eq!(Lang::Hant.among("骁龙", "驍龍", "Snapdragon"), "驍龍");
    }

    #[test]
    fn a_count_takes_the_form_the_language_gives_it() {
        use PluralCategory::{Few, Many, One, Other};
        assert_eq!([1, 2, 21].map(|n| Lang::En.plural(n)), [One, Other, Other]);
        assert_eq!([1, 2, 5, 11, 21, 22, 25].map(|n| Lang::Ru.plural(n)), [One, Few, Many, Many, One, Few, Many]);
        assert_eq!([1, 2, 5, 22].map(|n| Lang::Pl.plural(n)), [One, Few, Many, Few]);
        assert_eq!([0, 1, 2].map(|n| Lang::Fr.plural(n)), [One, One, Other], "French counts none as one");
        assert_eq!([1, 2].map(|n| Lang::Hans.plural(n)), [Other, Other], "Chinese has one form");
        assert_eq!([1, 2].map(|n| Lang::Ja.plural(n)), [Other, Other]);
    }

    #[test]
    fn letters_are_made_small_as_the_language_makes_them() {
        assert_eq!(Lang::En.lowercase("I"), "i");
        assert_eq!(Lang::Tr.lowercase("I"), "ı", "the dotless i of Turkish");
        assert_eq!(Lang::Tr.lowercase("İ"), "i");
        assert_eq!(Lang::Ru.lowercase("Н"), "н");
    }
}
