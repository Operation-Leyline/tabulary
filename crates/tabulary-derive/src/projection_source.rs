//! Field-local hooks, shared by Record and Document without decoding the source.
use crate::validate::Record;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

pub fn fields(s: &Record) -> TokenStream {
    let (id, path) = (&s.ident, &s.crate_path);
    let generics = crate::document_validate::bounds(s);
    let (ig, tg, wc) = generics.split_for_impl();
    let vis = &s.vis;
    let parts = format_ident!("__Tabulary{}ProjectionParts", s.prefix);
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    let mut part_types = Vec::new();
    let mut part_values = Vec::new();
    let mut preparations = Vec::new();
    for (i, f) in s.fields.iter().enumerate() {
        let (field, ty, name) = (&f.ident, &f.ty, &f.name);
        let output = if f.optional {
            quote!(::core::option::Option<#ty>)
        } else {
            quote!(#ty)
        };
        if crate::key_expand::template(s, i).is_some() {
            let local = format_ident!("__tabulary_projection_key_{i}", span = Span::mixed_site());
            let encode = crate::key_expand::encode(s, i);
            preparations.push(quote! {
                let #local = ::core::option::Option::Some((|| {
                    let #cx = #path::EncodeContext::new(<Self as #path::Document>::NAME);
                    let mut #item = #path::Item::new();
                    #encode
                    match #item.remove(#name) {
                        ::core::option::Option::Some(#path::__private::AttributeValue::S(value)) => ::core::result::Result::Ok(value),
                        _ => ::core::result::Result::Err(#cx.error(#path::EncodeFault::Custom { code: "invalid_projection_default" })),
                    }
                })());
            });
            part_types.push(
                quote!(::core::option::Option<::core::result::Result<#output, #path::EncodeError>>),
            );
            part_values.push(quote!(#local));
        } else {
            part_types.push(quote!(::core::option::Option<#output>));
            part_values.push(quote!(::core::option::Option::Some(self.#field)));
        }
    }
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
        let slot = syn::Index::from(i);
        let take = if crate::key_expand::template(s, i).is_some() {
            quote!(value.map_err(|error| #child.error(match error.kind {
                #path::EncodeFault::KeyFormat(fault) => #path::DecodeFault::KeyFormat(fault),
                _ => #path::DecodeFault::Custom { code: "invalid_projection_default" },
            })))
        } else { quote!(::core::result::Result::Ok(value)) };
        quote_spanned!(f.ident.span()=>
            impl #ig #path::projection::SourceField<#i, #output, #path::Attr<Self, #logical, #presence, #access>> for #id #tg #wc {
                const ATTRIBUTES: &'static [&'static str] = #names;
                fn decode(#item: &#path::Item, #cx: &mut #path::DecodeContext) -> ::core::result::Result<#output, #path::DecodeError> {
                    let #cx = #cx.in_document(<Self as #path::Document>::NAME);
                    #decode
                }
                fn take(parts: &mut Self::Parts, #cx: &mut #path::DecodeContext) -> ::core::result::Result<#output, #path::DecodeError> {
                    let value = parts.#slot.take().ok_or_else(|| #cx.error(#path::DecodeFault::Custom { code: "projection_field_consumed" }))?;
                    #take
                }
            }
        )
    });
    quote! {
        #[doc(hidden)]
        #vis struct #parts #generics (#(#part_types,)*) #wc;
        impl #ig #path::projection::ProjectionSource for #id #tg #wc {
            type Parts = #parts #tg;
            fn into_parts(self) -> Self::Parts { #(#preparations)* #parts (#(#part_values,)*) }
        }
        #(#fields)*
    }
}
