//! Reindex uses the checked index IR and the existing template formatter.
use crate::{index_validate::Index, key_expand as key, sparse_expand, validate::Record};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

fn inputs(record: &Record, index: &Index) -> Vec<usize> {
    let mut inputs = Vec::new();
    for i in std::iter::once(index.partition)
        .chain(index.sort)
        .flat_map(|i| sparse_expand::inputs(record, i))
    {
        if !inputs.contains(&i) {
            inputs.push(i);
        }
    }
    inputs
}
fn synthetic(record: &Record, index: &Index) -> Vec<usize> {
    std::iter::once(index.partition)
        .chain(index.sort)
        .filter(|i| key::template(record, *i).is_some() && !key::primary(record, *i))
        .collect()
}
fn encode_source(record: &Record, i: usize, receiver: TokenStream) -> TokenStream {
    let path = &record.crate_path;
    let cx = format_ident!("cx", span = Span::mixed_site());
    let item = format_ident!("item", span = Span::mixed_site());
    let f = &record.fields[i];
    let (id, name) = (&f.ident, &f.name);
    let encode = if let Some(max) = f
        .key_limit
        .into_iter()
        .chain(key::index_limit(record, i))
        .min()
    {
        quote!(#path::__private::encode_key(value, &mut #cx.at(#path::PathSegment::Attribute(#name.into())), #max)?)
    } else {
        quote!(#path::Value::encode_value(value, &mut #cx.at(#path::PathSegment::Attribute(#name.into())))?)
    };
    let write = quote!(#item.insert(#name.into(), #encode););
    if f.optional {
        quote!(if let ::core::option::Option::Some(value) = &#receiver.#id { #write })
    } else {
        quote!({ let value = &#receiver.#id; #write })
    }
}
pub fn expand(record: &Record, index: &Index, peer: &syn::Ident) -> TokenStream {
    let (path, owner, vis) = (&record.crate_path, &record.ident, &record.vis);
    let generics = crate::document_validate::bounds(record);
    let (ig, tg, wc) = generics.split_for_impl();
    let input = format_ident!("{}Inputs", peer);
    let key_type = format_ident!("{}Key", record.prefix);
    let fields = inputs(record, index);
    let keys = synthetic(record, index);
    let dependencies = record.indexes.iter().map(|i| {
        let fields = inputs(record, i);
        quote!(&[#(#fields),*])
    });
    let synthetic_dependencies = record.indexes.iter().map(|i| {
        let fields = synthetic(record, i);
        quote!(&[#(#fields),*])
    });
    let ids: Vec<_> = fields.iter().map(|i| &record.fields[*i].ident).collect();
    let types: Vec<_> = fields
        .iter()
        .map(|i| {
            let f = &record.fields[*i];
            let ty = &f.ty;
            if f.optional {
                quote!(::core::option::Option<#ty>)
            } else {
                quote!(#ty)
            }
        })
        .collect();
    let visibility = fields.iter().map(|i| &record.fields[*i].vis);
    let mut marker = format_ident!("__tabulary_marker");
    while ids.contains(&&marker) {
        marker = format_ident!("{}_", marker);
    }
    let generic = !record.generics.params.is_empty();
    let marker_decl =
        generic.then(|| quote!(#marker: ::core::marker::PhantomData<fn() -> #owner #tg>,));
    let marker_value = generic.then(|| quote!(#marker: ::core::marker::PhantomData,));
    let writes = fields
        .iter()
        .map(|i| encode_source(record, *i, quote!(self)));
    let present = fields
        .iter()
        .filter(|i| record.fields[**i].optional)
        .map(|i| {
            let id = &record.fields[*i].ident;
            quote!(self.#id.is_some())
        });
    let member = quote!((true #(&& #present)*));
    let key_writes = keys.iter().map(|i| {
        let validate = sparse_expand::validate_sources(record, *i, true);
        let body = key::encode_body(record, *i);
        let optional: Vec<_> = sparse_expand::inputs(record, *i).filter(|i| record.fields[*i].optional).map(|i| &record.fields[i].ident).collect();
        let body = if optional.is_empty() { body } else {
            quote!(if let (#(::core::option::Option::Some(#optional),)*) = (#(&self.#optional,)*) { #body })
        };
        quote!(#validate if #member { #body })
    });
    let primary_inputs = key::inputs(record);
    let primary_fields: Vec<_> = fields
        .iter()
        .filter(|i| primary_inputs.contains(i))
        .copied()
        .collect();
    let primary_writes = primary_fields
        .iter()
        .map(|i| encode_source(record, *i, quote!(key)));
    let decode_key = (!primary_fields.is_empty()).then(|| quote! {
        let key = <#key_type #tg as #path::RecordKey<#owner #tg>>::decode_key(key)
            .map_err(|_| #path::BuildError::OperationConstraint { code: "invalid_reindex_key" })?;
    });
    let cx = format_ident!("cx", span = Span::mixed_site());
    let item = format_ident!("item", span = Span::mixed_site());
    quote! {
        #vis struct #input #generics #wc { #(#visibility #ids: #types,)* #marker_decl }
        impl #ig #input #tg #wc {
            #vis fn new(#(#ids: #types),*) -> Self { Self { #(#ids,)* #marker_value } }
            fn __tabulary_encode(&self) -> ::core::result::Result<#path::Item, #path::EncodeError> {
                let #cx = #path::EncodeContext::new(::core::stringify!(#owner));
                let mut #item = #path::Item::new();
                #(#writes)* #(#key_writes)*
                let _ = (&#cx, &mut #item);
                ::core::result::Result::Ok(#item)
            }
        }
        impl #ig #path::ReindexSchema<#owner #tg> for #peer #tg #wc {
            type Inputs = #input #tg;
            const INPUTS: &'static [usize] = &[#(#fields),*];
            const SYNTHETIC: &'static [usize] = &[#(#keys),*];
            const DEPENDENCIES: &'static [&'static [usize]] = &[#(#dependencies),*];
            const SYNTHETIC_DEPENDENCIES: &'static [&'static [usize]] = &[#(#synthetic_dependencies),*];
            fn encode(inputs: &Self::Inputs) -> ::core::result::Result<#path::Item, #path::EncodeError> { inputs.__tabulary_encode() }
            fn primary(key: &#path::Item) -> ::core::result::Result<#path::Item, #path::BuildError> {
                #decode_key
                let #cx = #path::EncodeContext::new(::core::stringify!(#owner));
                let mut #item = #path::Item::new();
                #(#primary_writes)*
                let _ = (key, #cx, &mut #item);
                ::core::result::Result::Ok(#item)
            }
        }
    }
}
