use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::{env, fs};
use toml::{Table, Value};

/// A language the pages speak: what the code calls it, the locale its file
/// in `locales` is named after, and what it is.
struct Language {
    variant: &'static str,
    tag: &'static str,
    doc: &'static str,
}

const fn language(variant: &'static str, tag: &'static str, doc: &'static str) -> Language {
    Language { variant, tag, doc }
}

/// The languages, in the order the language page lists them.
const LANGUAGES: &[Language] = &[
    language("Hans", "zh-Hans", "Chinese in simplified characters, as mainland China writes it."),
    language("Hant", "zh-Hant", "Chinese in traditional characters, as Taiwan writes it."),
    language("En", "en", "English."),
    language("Tr", "tr", "Turkish."),
    language("Ru", "ru", "Russian."),
    language("Uk", "uk", "Ukrainian."),
    language("Es", "es", "Spanish, as Spain and Latin America both read it."),
    language("PtBr", "pt-BR", "Portuguese, as Brazil writes it."),
    language("Id", "id", "Indonesian."),
    language("Vi", "vi", "Vietnamese."),
    language("De", "de", "German."),
    language("Fr", "fr", "French."),
    language("It", "it", "Italian."),
    language("Pl", "pl", "Polish."),
    language("Ja", "ja", "Japanese."),
];

/// The language every other is held to: each has its texts, with the same
/// values named in them, and the code that is written shows its words.
const REFERENCE: &str = "en";

/// The count a text that differs by number is chosen and written with.
const COUNT: &str = "n";

/// What a language file gives under a name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Message {
    /// A text, with the values it names in braces.
    Text(String),
    /// A text for each of the forms the language's numbers take, by the
    /// form's name in CLDR.
    Plural(BTreeMap<String, String>),
    /// Something the language does or does not do.
    Flag(bool),
}

/// A language file: the language's name written in it, and its messages by
/// the tables they are under and their own names.
struct Catalog {
    name: String,
    messages: BTreeMap<Vec<String>, Message>,
}

/// The forms CLDR knows numbers to take, as language files and
/// `PluralCategory` name them.
const FORMS: [(&str, PluralCategory); 6] = [
    ("zero", PluralCategory::Zero),
    ("one", PluralCategory::One),
    ("two", PluralCategory::Two),
    ("few", PluralCategory::Few),
    ("many", PluralCategory::Many),
    ("other", PluralCategory::Other),
];

fn main() {
    let folder = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales");
    println!("cargo:rerun-if-changed={}", folder.display());
    let known: BTreeSet<String> = LANGUAGES.iter().map(|l| format!("{}.toml", l.tag)).collect();
    for entry in fs::read_dir(&folder).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(known.contains(&name), "locales/{name} is not the file of a language the pages speak");
    }
    let catalogs: Vec<Catalog> = LANGUAGES.iter().map(|l| read(&folder.join(format!("{}.toml", l.tag)), l.tag)).collect();
    let reference = &catalogs[LANGUAGES.iter().position(|l| l.tag == REFERENCE).unwrap()];
    for (language, catalog) in LANGUAGES.iter().zip(&catalogs) {
        check(language, catalog, reference);
    }
    let out = env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out).join("lang.rs"), languages(&catalogs).to_string()).unwrap();
    fs::write(Path::new(&out).join("texts.rs"), texts(&catalogs, reference).to_string()).unwrap();
}

/// Read the file of the language `tag`.
fn read(path: &Path, tag: &str) -> Catalog {
    let source = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut table: Table = source.parse().unwrap_or_else(|e| panic!("{tag}.toml: {e}"));
    let name = match table.remove("name") {
        Some(Value::String(name)) => name,
        other => panic!("{tag}.toml: `name` is to be the language's name, written in it, and is {other:?}"),
    };
    let mut messages = BTreeMap::new();
    for (key, value) in &table {
        match value {
            Value::Table(inner) if !is_plural(inner) => flatten(tag, &mut vec![key.clone()], inner, &mut messages),
            _ => panic!("{tag}.toml: `{key}` is to be a table of texts"),
        }
    }
    Catalog { name, messages }
}

/// Whether a table gives one text in the forms numbers take, and not texts
/// under their names.
fn is_plural(table: &Table) -> bool {
    table.contains_key("other") && table.iter().all(|(key, value)| FORMS.iter().any(|(form, _)| *form == key.as_str()) && value.is_str())
}

fn flatten(tag: &str, path: &mut Vec<String>, table: &Table, out: &mut BTreeMap<Vec<String>, Message>) {
    for (key, value) in table {
        path.push(key.clone());
        let at = path.join(".");
        assert!(syn::parse_str::<syn::Ident>(key).is_ok(), "{tag}.toml: `{at}` is not a name the code can call it by");
        assert!(*key == key.to_lowercase(), "{tag}.toml: `{at}` is to be written in small letters");
        match value {
            Value::String(text) => {
                out.insert(path.clone(), Message::Text(text.clone()));
            }
            Value::Boolean(flag) => {
                out.insert(path.clone(), Message::Flag(*flag));
            }
            Value::Table(inner) if is_plural(inner) => {
                let forms = inner.iter().map(|(form, text)| (form.clone(), text.as_str().unwrap().to_string())).collect();
                out.insert(path.clone(), Message::Plural(forms));
            }
            Value::Table(inner) => flatten(tag, path, inner, out),
            other => panic!("{tag}.toml: `{at}` is neither a text, a switch nor a table, but {other:?}"),
        }
        path.pop();
    }
}

/// The values a text names in braces, in the order it first names each. A
/// brace written twice stands for itself.
fn values(tag: &str, at: &str, text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '{' => {
                let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
                assert!(syn::parse_str::<syn::Ident>(&name).is_ok(), "{tag}.toml: `{at}` names {{{name}}}, which is not a value's name");
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            '}' => panic!("{tag}.toml: `{at}` closes a brace it did not open"),
            _ => {}
        }
    }
    names
}

/// The values a message names, in the order it first names each: a text
/// that differs by number names its count first, and the rest as its form
/// for every other number does.
fn message_values(tag: &str, at: &str, message: &Message) -> Vec<String> {
    match message {
        Message::Text(text) => values(tag, at, text),
        Message::Plural(forms) => {
            let mut names = vec![COUNT.to_string()];
            for text in std::iter::once(&forms["other"]).chain(forms.values()) {
                for name in values(tag, at, text) {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            names
        }
        Message::Flag(_) => Vec::new(),
    }
}

/// The forms the numbers of the language `tag` take.
fn forms_of(tag: &str) -> BTreeSet<String> {
    let locale = Locale::try_from_str(tag).unwrap_or_else(|e| panic!("{tag} is not a locale: {e}"));
    let rules = PluralRules::try_new_cardinal(locale.into()).unwrap_or_else(|e| panic!("the plural rules of {tag}: {e}"));
    rules.categories().map(|c| FORMS.iter().find(|(_, form)| *form == c).unwrap().0.to_string()).collect()
}

/// Hold a language's file to the reference's: the same messages, each of
/// the same kind and naming the same values, and a text that differs by
/// number written in just the forms the language's numbers take.
fn check(language: &Language, catalog: &Catalog, reference: &Catalog) {
    let tag = language.tag;
    let name = |path: &Vec<String>| path.join(".");
    let missing: Vec<String> = reference.messages.keys().filter(|k| !catalog.messages.contains_key(*k)).map(name).collect();
    assert!(missing.is_empty(), "{tag}.toml lacks what {REFERENCE}.toml has: {}", missing.join(", "));
    let extra: Vec<String> = catalog.messages.keys().filter(|k| !reference.messages.contains_key(*k)).map(name).collect();
    assert!(extra.is_empty(), "{tag}.toml has what {REFERENCE}.toml lacks: {}", extra.join(", "));
    let forms = forms_of(tag);
    for (path, message) in &catalog.messages {
        let at = name(path);
        let theirs = &reference.messages[path];
        assert!(
            std::mem::discriminant(message) == std::mem::discriminant(theirs),
            "{tag}.toml: `{at}` is not of the kind it is in {REFERENCE}.toml"
        );
        let sorted = |m: &Message, tag: &str| message_values(tag, &at, m).into_iter().collect::<BTreeSet<String>>();
        let (named, wanted) = (sorted(message, tag), sorted(theirs, REFERENCE));
        assert!(named == wanted, "{tag}.toml: `{at}` names {named:?}, and in {REFERENCE}.toml it names {wanted:?}");
        if let Message::Plural(written) = message {
            let written: BTreeSet<String> = written.keys().cloned().collect();
            assert!(written == forms, "{tag}.toml: `{at}` is written for {written:?}, and the language's numbers take the forms {forms:?}");
        }
    }
}

/// The languages as the code has them.
fn languages(catalogs: &[Catalog]) -> TokenStream {
    let count = LANGUAGES.len();
    let variants: Vec<_> = LANGUAGES.iter().map(|l| format_ident!("{}", l.variant)).collect();
    let docs = LANGUAGES.iter().map(|l| l.doc);
    let tags = LANGUAGES.iter().map(|l| l.tag);
    let names = catalogs.iter().map(|c| c.name.as_str());
    quote! {
        /// A language the pages speak.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Lang {
            #(#[doc = #docs] #variants,)*
        }

        impl Lang {
            /// How many languages the pages speak.
            pub const COUNT: usize = #count;

            /// The languages, in the order the language page lists them.
            pub const ALL: [Lang; #count] = [#(Lang::#variants),*];

            /// The language as a locale, which [`Lang::for_locale`] reads
            /// back, and which its file of texts is named after.
            pub const fn tag(self) -> &'static str {
                match self {
                    #(Lang::#variants => #tags,)*
                }
            }

            /// The language's name, written in it.
            pub const fn name(self) -> &'static str {
                match self {
                    #(Lang::#variants => #names,)*
                }
            }
        }
    }
}

/// The messages under one table: its own, and the tables under it.
#[derive(Default)]
struct Module {
    messages: Vec<TokenStream>,
    modules: BTreeMap<String, Module>,
}

impl Module {
    fn tokens(&self) -> TokenStream {
        let messages = &self.messages;
        let modules = self.modules.iter().map(|(name, module)| {
            let (name, inner) = (format_ident!("{name}"), module.tokens());
            quote! { pub mod #name { #inner } }
        });
        quote! { #(#messages)* #(#modules)* }
    }
}

/// The texts as the code has them: a table as a module, a text that names
/// no value as a constant holding it in every language, one that names
/// values as a function that writes it with them, and a switch as a
/// constant holding how each language has it.
fn texts(catalogs: &[Catalog], reference: &Catalog) -> TokenStream {
    let mut root = Module::default();
    for (path, theirs) in &reference.messages {
        let (key, tables) = path.split_last().unwrap();
        let module = tables.iter().fold(&mut root, |module, table| module.modules.entry(table.clone()).or_default());
        let at = path.join(".");
        let each: Vec<&Message> = catalogs.iter().map(|c| &c.messages[path]).collect();
        module.messages.push(match theirs {
            Message::Flag(_) => {
                let name = format_ident!("{}", key.to_uppercase());
                let flags = each.iter().map(|m| matches!(m, Message::Flag(true)));
                quote! { pub const #name: [bool; crate::lang::Lang::COUNT] = [#(#flags),*]; }
            }
            Message::Text(text) if values(REFERENCE, &at, text).is_empty() => {
                let name = format_ident!("{}", key.to_uppercase());
                let written = each.iter().map(|m| match m {
                    Message::Text(text) => unbraced(text),
                    _ => unreachable!("checked to be a text"),
                });
                quote! {
                    #[doc = #text]
                    pub const #name: crate::lang::Text = [#(#written),*];
                }
            }
            Message::Text(text) => {
                let name = format_ident!("{key}");
                let given = values(REFERENCE, &at, text).into_iter().map(|v| format_ident!("{v}"));
                let arms = LANGUAGES.iter().zip(&each).map(|(l, m)| {
                    let variant = format_ident!("{}", l.variant);
                    let Message::Text(text) = m else { unreachable!("checked to be a text") };
                    quote! { crate::lang::Lang::#variant => format!(#text) }
                });
                quote! {
                    #[doc = #text]
                    pub fn #name(l: crate::lang::Lang, #(#given: impl ::core::fmt::Display),*) -> String {
                        match l {
                            #(#arms,)*
                        }
                    }
                }
            }
            Message::Plural(forms) => {
                let name = format_ident!("{key}");
                let doc = &forms["other"];
                let count = format_ident!("{COUNT}");
                let given = message_values(REFERENCE, &at, theirs).into_iter().skip(1).map(|v| format_ident!("{v}"));
                let arms = LANGUAGES.iter().zip(&each).flat_map(|(l, m)| {
                    let variant = format_ident!("{}", l.variant);
                    let Message::Plural(forms) = m else { unreachable!("checked to differ by number") };
                    let special = forms.iter().filter(|(form, _)| *form != "other").map(|(form, text)| {
                        let form = format_ident!("{}", FORMS.iter().find(|(name, _)| *name == form.as_str()).map(|(_, c)| format!("{c:?}")).unwrap());
                        let written = written(l.tag, &at, text);
                        quote! { (crate::lang::Lang::#variant, ::icu_plurals::PluralCategory::#form) => #written }
                    });
                    let other = written(l.tag, &at, &forms["other"]);
                    special.chain(std::iter::once(quote! { (crate::lang::Lang::#variant, _) => #other })).collect::<Vec<_>>()
                });
                quote! {
                    #[doc = #doc]
                    pub fn #name(l: crate::lang::Lang, #count: u64, #(#given: impl ::core::fmt::Display),*) -> String {
                        match (l, l.plural(#count)) {
                            #(#arms,)*
                        }
                    }
                }
            }
        });
    }
    root.tokens()
}

/// A text that names no value, as it reads: with each brace written twice
/// as the one it stands for.
fn unbraced(text: &str) -> String {
    text.replace("{{", "{").replace("}}", "}")
}

/// The code that writes one form of a text that differs by number: made
/// of the values it names, or as it stands where it names none.
fn written(tag: &str, at: &str, text: &str) -> TokenStream {
    match values(tag, at, text).is_empty() {
        true => {
            let text = unbraced(text);
            quote! { String::from(#text) }
        }
        false => quote! { format!(#text) },
    }
}
