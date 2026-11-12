//! Field-local hooks, shared by Record and Document without decoding the source.
use crate::validate::Record;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

pub fn fields(s: &Record) -> TokenStream {
    let (id, path) = (&s.ident, &s.crate_path);
    let generics = crate::document_validate::bounds(s);
    let (ig, tg, wc) = generics.split_for_impl();
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    let fields = s.fields.iter().enumerate().map(|(i, f)| {
        let (ty, name) = (&f.ty, &f.name);
        let presence = if f.optional { quote!(#path::Optional) } else { quote!(#path::Required) };
        let access = if f.flatten { quote!(#path::DerivedOnly) } else { crate::key_expand::access(s, i) };
        let logical = if f.flatten { quote!(<#ty as #path::Variants>::Tag) } else { quote!(#ty) };
        let output = if f.optional { quote!(::core::option::Option<#ty>) } else { quote!(#ty) };
        let names = if f.flatten { quote!(<#ty as #path::Variants>::ATTRIBUTES) } else { quote!(&[#name]) };
        let child = quote!(#cx.at(#path::PathSegment::Attribute(#name.into())));
        let decode = if crate::key_expand::template(s, i).is_some() {
            let parse = crate::key_expand::decode(s, i);
            let text = format_ident!("__tabulary_text_{i}", span = Span::mixed_site());
            quote!({ #parse ::core::result::Result::Ok(#text) })
        }
        else if f.flatten { quote!(<#ty as #path::Variants>::decode_flattened(#item, &mut #cx.clone())) }
        else if let Some(max) = f.key_limit { quote!(#path::__private::decode_key(#item.get(#name), &mut #child, #max)) }
        else if f.optional { quote!(#path::field::decode_optional::<#ty, #path::Native<#ty>>(#item.get(#name), &mut #child)) }
        else { quote!(#path::field::decode_required::<#ty, #path::Native<#ty>>(#item.get(#name), &mut #child)) };
        let decode = if let Some(default) = &f.default {
            quote!(if !#item.contains_key(#name) { let default: fn() -> #output = #default; return ::core::result::Result::Ok(default()); } #decode)
        } else { decode };
        quote_spanned!(f.ident.span()=>
            impl #ig #path::projection::SourceField<#i> for #id #tg #wc {
                type Handle = #path::Attr<Self, #logical, #presence, #access>;
                type Output = #output;
                type Presence = #presence;
                const ATTRIBUTES: &'static [&'static str] = #names;
                fn decode(#item: &#path::Item, #cx: &mut #path::DecodeContext) -> ::core::result::Result<Self::Output, #path::DecodeError> {
                    let #cx = #cx.in_document(<Self as #path::Document>::NAME);
                    #decode
                }
            }
        )
    });
    quote!(#(#fields)*)
}
