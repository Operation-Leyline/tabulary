#![forbid(unsafe_code)]

mod expand;
mod parse;
mod validate;

#[proc_macro_derive(Record, attributes(tabulary))]
pub fn record(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    validate::record(input)
        .map(|record| expand::record(&record))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
