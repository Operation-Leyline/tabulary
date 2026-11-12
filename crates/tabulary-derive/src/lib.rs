#![forbid(unsafe_code)]

mod document_expand;
mod document_parse;
mod document_validate;
mod expand;
mod parse;
mod string_enum_expand;
mod string_enum_parse;
mod string_enum_validate;
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

/// Derive an owned strict map codec without table or key metadata.
#[proc_macro_derive(Document, attributes(tabulary))]
pub fn document(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    document_validate::document(input)
        .map(|schema| document_expand::document(&schema))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
