//! What the pages say, in every language they speak: each language's file
//! in `locales`, as the build script writes it out. A table there is a
//! module here; a text that names no value is a constant holding it in
//! every language, for [`Lang::pick`](crate::lang::Lang::pick) to choose
//! from, and one that names values in braces is a function that writes it
//! with them.

include!(concat!(env!("OUT_DIR"), "/texts.rs"));
