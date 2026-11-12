//! Query-facing primary metadata; templates share the existing logical codecs.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::{key_expand, validate::Record};

pub fn primary(record: &Record) -> TokenStream {
    let (path, owner) = (&record.crate_path, &record.ident);
    let peer = format_ident!("{}Primary", record.prefix);
    let generics = crate::document_validate::bounds(record);
    let (ig, tg, wc) = generics.split_for_impl();
    let mut codecs = Vec::new();
    let mut key_type = |i, suffix| {
        if key_expand::template(record, i).is_some() {
            let codec = format_ident!("{}{}Codec", record.prefix, suffix);
            let (logical, declaration) = key_expand::template_codec(record, i, &codec);
            codecs.push(declaration);
            (logical, quote!(#codec #tg))
        } else {
            let ty = &record.fields[i].ty;
            (quote!(#ty), quote!(#path::Native<#ty>))
        }
    };
    let (partition, partition_codec) = key_type(record.partition, "Partition");
    let sort = record.sort.map_or_else(
        || quote!(#path::NoSort),
        |i| {
            let (ty, codec) = key_type(i, "Sort");
            quote!(#path::SortKey<#ty, #codec>)
        },
    );
    quote! {
        #(#codecs)*
        impl #ig #path::KeySchema<#owner #tg> for #peer #tg #wc {
            type Partition = #partition;
            type PartitionCodec = #partition_codec;
            type Sort = #sort;
            const KEYS: #path::KeyDefinition = <#owner #tg as #path::Record>::SCHEMA.primary;
        }
        impl #ig #path::PrimarySchema<#owner #tg> for #peer #tg #wc {}
    }
}
