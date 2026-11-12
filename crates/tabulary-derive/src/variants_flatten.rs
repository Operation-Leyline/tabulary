//! The only integration hooks needed by the struct derives.
use crate::validate::Record;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

pub fn field(schema: &Record, i: usize) -> (TokenStream, TokenStream, TokenStream) {
    let f = &schema.fields[i];
    let (id, ty, path) = (&f.ident, &f.ty, &schema.crate_path);
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    let local = format_ident!("__tabulary_field_{i}", span = Span::mixed_site());
    (
        quote!(#item.extend(<#ty as #path::Document>::encode_map(&self.#id, &mut #cx.clone())?);),
        quote!(let #local = <#ty as #path::Variants>::decode_flattened(#item, &mut #cx.clone())?;),
        quote!(<#ty as #path::Variants>::Tag),
    )
}

pub fn validation(schema: &Record) -> TokenStream {
    let Some(f) = schema.fields.iter().find(|f| f.flatten) else {
        return quote!();
    };
    let (ty, path) = (&f.ty, &schema.crate_path);
    let names = schema.fields.iter().filter(|f| !f.flatten).map(|f| &f.name);
    quote_spanned!(f.ident.span()=> const { ::core::assert!(#path::__private::disjoint(&[#(#names,)*], <#ty as #path::Variants>::ATTRIBUTES), "flattened variant attribute overlaps a common field") };)
}

pub fn descriptor(schema: &Record, i: usize) -> TokenStream {
    let (ty, path) = (&schema.fields[i].ty, &schema.crate_path);
    let tag = quote!(<#ty as #path::Variants>::Tag);
    quote!(#path::__private::field::<Self, #tag, #path::Required, #path::DerivedOnly, #path::Native<#tag>>(<#ty as #path::Variants>::TAG, #i).descriptor())
}

pub fn known(schema: &Record) -> TokenStream {
    let path = &schema.crate_path;
    let ty = schema.fields.iter().find(|f| f.flatten).map(|f| &f.ty);
    ty.map_or_else(
        || quote!(false),
        |ty| quote!(<#ty as #path::Variants>::ATTRIBUTES.contains(&name.as_str())),
    )
}

pub fn transition(schema: &Record) -> TokenStream {
    let Some((i, field)) = schema.fields.iter().enumerate().find(|(_, f)| f.flatten) else {
        return quote!();
    };
    let (id, ty, path) = (&schema.ident, &field.ty, &schema.crate_path);
    let generics = crate::document_validate::bounds(schema);
    let (ig, tg, wc) = generics.split_for_impl();
    quote! {
        impl #ig #path::VariantRecord for #id #tg #wc {
            type Group = #ty;
        }
        impl #ig #path::FlattenedVariants<#ty> for #id #tg #wc {
            const FIELD: usize = #i;
        }
    }
}
