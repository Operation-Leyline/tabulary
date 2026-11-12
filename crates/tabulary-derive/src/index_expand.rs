use crate::{index_validate::Index, parse, validate::Record};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

pub fn definition(record: &Record, index: &Index) -> TokenStream {
    let (path, name, partition) = (&record.crate_path, &index.name, index.partition);
    let sort = index.sort.map_or_else(
        || quote!(::core::option::Option::None),
        |s| quote!(::core::option::Option::Some(#s)),
    );
    let kind = if index.local {
        quote!(#path::IndexKind::Local)
    } else {
        quote!(#path::IndexKind::Global)
    };
    let projection = if let Some(names) = &index.include {
        quote!(#path::IndexProjection::Include(&[#(#names),*]))
    } else if index.all {
        quote!(#path::IndexProjection::All)
    } else {
        quote!(#path::IndexProjection::KeysOnly)
    };
    quote!(#path::IndexDefinition { name: #name, kind: #kind, keys: #path::__private::keys(#partition, #sort), projection: #projection })
}
pub fn expand(record: &Record) -> TokenStream {
    if record.indexes.is_empty() {
        return quote!();
    }
    let (path, owner, vis) = (&record.crate_path, &record.ident, &record.vis);
    let generics = crate::document_validate::bounds(record);
    let (ig, tg, wc) = generics.split_for_impl();
    let indexes = format_ident!("{}Indexes", record.prefix);
    let mut handles = Vec::new();
    let mut peers = Vec::new();
    for index in &record.indexes {
        let peer = format_ident!(
            "{}{}",
            record.prefix,
            index.ident.to_string().trim_start_matches("r#")
        );
        let handle = format_ident!(
            "{}",
            parse::rename(
                index.ident.to_string().trim_start_matches("r#"),
                "SCREAMING_SNAKE_CASE"
            )
        );
        let partition = &record.fields[index.partition].ty;
        let partition_codec = quote!(#path::Native<#partition>);
        let sort = index.sort.map_or_else(
            || quote!(#path::NoSort),
            |i| {
                let ty = &record.fields[i].ty;
                quote!(#path::SortKey<#ty>)
            },
        );
        let definition = definition(record, index);
        handles.push(quote!(#vis const #handle: #peer #tg = #peer(::core::marker::PhantomData);));
        peers.push(quote! {
            #vis struct #peer #generics (::core::marker::PhantomData<fn() -> #owner #tg>) #wc;
            impl #ig ::core::marker::Copy for #peer #tg #wc {}
            impl #ig ::core::clone::Clone for #peer #tg #wc { fn clone(&self) -> Self { *self } }
            impl #ig #path::IndexSchema<#owner #tg> for #peer #tg #wc {
                type Partition = #partition;
                type PartitionCodec = #partition_codec;
                type Sort = #sort;
                const DEFINITION: #path::IndexDefinition = #definition;
            }
        });
    }
    quote!(impl #ig #indexes #tg #wc { #(#handles)* } #(#peers)*)
}
