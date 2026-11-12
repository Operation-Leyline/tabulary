#![forbid(unsafe_code)]

mod document_expand;
mod document_parse;
mod document_validate;
mod expand;
mod index_expand;
mod index_parse;
mod index_validate;
mod key_expand;
mod parse;
mod projection_expand;
mod projection_parse;
mod projection_source;
mod query_expand;
mod reindex_expand;
mod sparse_expand;
mod string_enum_expand;
mod string_enum_parse;
mod string_enum_validate;
mod template;
mod validate;
mod variants_expand;
mod variants_flatten;
mod variants_parse;
mod variants_validate;

/// Derive a read-only view using typed source-field handles.
#[proc_macro_derive(Projection, attributes(tabulary))]
pub fn projection(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    projection_parse::projection(input)
        .map(|schema| projection_expand::projection(&schema))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive a strict discriminator-selected map, optionally flattened into a document.
#[proc_macro_derive(Variants, attributes(tabulary))]
pub fn variants(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    variants_validate::variants(input)
        .map(|schema| variants_expand::variants(&schema))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

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
