use crate::projection_parse::Projection;
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};

pub fn projection(s: &Projection) -> TokenStream {
    let (id, source, path) = (&s.input.ident, &s.source, &s.path);
    let mut selections = Vec::new();
    let mut reads = Vec::new();
    for field in &s.fields {
        let (name, ty, handle) = (&field.id, &field.ty, &field.handle);
        let index = quote!({ (#handle).descriptor().index });
        selections.push(quote_spanned!(name.span()=> #path::projection::append(&mut names, #path::projection::select::<#source, #index>(#handle)?)?;));
        let decode = if field.nested {
            let view = crate::validate::optional(ty).unwrap_or_else(|| ty.clone());
            selections.push(
                quote_spanned!(name.span()=> let _ = <#view as #path::Projection>::attributes()?;),
            );
            quote_spanned!(name.span()=> #path::projection::decode_nested::<#source, #index, #view>(item, &mut cx)?)
        } else {
            quote_spanned!(name.span()=> <#source as #path::projection::SourceField<#index>>::decode(item, &mut cx)?)
        };
        reads.push(quote_spanned!(name.span()=> #name: { let value: #ty = #decode; value }));
    }
    quote! {
        impl #path::Projection for #id {
            type Source = #source;
            fn attributes() -> ::core::result::Result<::std::vec::Vec<&'static str>, #path::BuildError> {
                let mut names = ::std::vec::Vec::new(); #(#selections)* ::core::result::Result::Ok(names)
            }
            fn decode_map(item: &#path::Item, cx: &mut #path::DecodeContext) -> ::core::result::Result<Self, #path::DecodeError> {
                let mut cx = cx.in_document(<#source as #path::Document>::NAME);
                <Self as #path::Projection>::attributes().map_err(|_| cx.error(#path::DecodeFault::Custom { code: "invalid_projection" }))?;
                ::core::result::Result::Ok(Self { #(#reads,)* })
            }
        }
    }
}
