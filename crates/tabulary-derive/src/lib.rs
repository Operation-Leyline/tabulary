#![forbid(unsafe_code)]

mod expand;
mod parse;
mod string_enum_expand;
mod string_enum_parse;
mod string_enum_validate;
mod template;
mod validate;

#[proc_macro_derive(Record, attributes(tabulary))]
pub fn record(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    validate::record(input)
        .map(|record| expand::record(&record))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive strict string-wire `Value` conversion for an enum of unit variants.
#[proc_macro_derive(StringEnum, attributes(tabulary))]
pub fn string_enum(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    string_enum_validate::string_enum(input)
        .map(|schema| string_enum_expand::string_enum(&schema))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
