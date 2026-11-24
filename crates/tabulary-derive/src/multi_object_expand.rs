//! Expand a checked multi-object enum into its runtime contract and members.

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};

use crate::multi_object_parse::{MultiObject, Pattern};

pub fn multi_object(object: &MultiObject) -> TokenStream {
    let MultiObject {
        ident,
        vis,
        crate_path: path,
        partition,
        sort,
        variants,
    } = object;
    let first = &variants[0].ty;
    let pattern = |p: &Pattern| match p {
        Pattern::Any => quote!(#path::KeyPattern::Any),
        Pattern::Prefix(text) => quote!(#path::KeyPattern::Prefix(#text)),
        Pattern::Exact(text) => quote!(#path::KeyPattern::Exact(#text)),
    };
    let (sort_field, sort_position) = match sort {
        Some(sort) => (
            quote!(#path::__private::multi_key::<#first>(#sort, 1, true),),
            quote!(::core::option::Option::Some(1)),
        ),
        None => (quote!(), quote!(::core::option::Option::None)),
    };
    let entries = variants.iter().map(|v| {
        let (name, ty) = (v.ident.to_string(), &v.ty);
        let (p, s) = (pattern(&v.partition), pattern(&v.sort));
        quote! {
            #path::MultiVariant {
                name: #name,
                record: <#ty as #path::Document>::NAME,
                keys: #path::KeyPatterns { partition: #p, sort: #s },
            }
        }
    });
    let decodes = variants.iter().enumerate().map(|(i, v)| {
        let (variant, ty) = (&v.ident, &v.ty);
        quote!(#i => <#ty as #path::Document>::decode_map(item, cx).map(Self::#variant),)
    });
    let encodes = variants.iter().map(|v| {
        let (variant, ty) = (&v.ident, &v.ty);
        quote! {
            Self::#variant(record) => <#ty as #path::Document>::encode_map(
                record,
                &mut #path::EncodeContext::new(<#ty as #path::Document>::NAME),
            ),
        }
    });
    let members = variants.iter().enumerate().map(|(i, v)| {
        let (variant, ty) = (&v.ident, &v.ty);
        let span = variant.span();
        quote_spanned! {span=>
            impl #path::MemberOf<#ident> for #ty {
                const VARIANT: usize = #i;
                const SEAL: __TabularyMemberSeal = __TabularyMemberSeal(());
                fn into_object(self) -> #ident { #ident::#variant(self) }
            }
            impl ::core::convert::From<#ty> for #ident {
                fn from(record: #ty) -> Self { Self::#variant(record) }
            }
            const _: () = {
                if let ::core::option::Option::Some(fault) = #path::member_fault::<#ident, #ty>() {
                    ::core::panic!("{}", fault);
                }
            };
        }
    });
    let names = variants.iter().map(|v| &v.ident);
    let name = ident.to_string();
    quote! {
        // The seal is declared in an anonymous scope with a private field, so
        // only this expansion can construct it and implement `MemberOf`.
        const _: () = {
        pub struct __TabularyMemberSeal(());
        impl #path::MultiObject for #ident {
            type MemberSeal = __TabularyMemberSeal;
            const SCHEMA: #path::RecordSchema = #path::__private::schema(
                #name,
                &[#path::__private::multi_key::<#first>(#partition, 0, false), #sort_field],
                #path::__private::keys(0, #sort_position),
            );
            const VARIANTS: &'static [#path::MultiVariant] = &[#(#entries),*];
            fn decode_variant(
                variant: usize,
                item: &#path::Item,
                cx: &mut #path::DecodeContext,
            ) -> ::core::result::Result<Self, #path::DecodeError> {
                match variant {
                    #(#decodes)*
                    _ => ::core::result::Result::Err(cx.error(#path::DecodeFault::UnknownVariant)),
                }
            }
        }
        const _: () = {
            if let ::core::option::Option::Some(fault) = #path::multi_object_fault::<#ident>() {
                ::core::panic!("{}", fault);
            }
        };
        #(#members)*
        impl #ident {
            /// Select the member from the stored keys, then decode it strictly.
            #vis fn from_item(item: &#path::Item) -> ::core::result::Result<Self, #path::DecodeError> {
                #path::decode_object::<Self>(item)
            }
            /// Encode the held record with its own document codec.
            #vis fn to_item(&self) -> ::core::result::Result<#path::Item, #path::EncodeError> {
                match self { #(#encodes)* }
            }
            /// The declared variant name of the held record.
            #vis fn variant_name(&self) -> &'static str {
                match self { #(Self::#names(_) => ::core::stringify!(#names),)* }
            }
        }
        };
    }
}
