//! Sparse membership is checked after every ordinary source has been decoded.
use crate::{key_expand as key, template::Part, validate::Record};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

pub fn inputs(record: &Record, field: usize) -> impl Iterator<Item = usize> + '_ {
    key::template(record, field)
        .map(|t| key::components(&t.parts).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![field])
        .into_iter()
}
pub fn conditional(record: &Record, field: usize) -> bool {
    !key::primary(record, field)
        && record
            .indexes
            .iter()
            .filter(|i| i.partition == field || i.sort == Some(field))
            .all(|i| i.sparse)
}
fn membership(record: &Record, field: usize, receiver: bool) -> TokenStream {
    let members = record
        .indexes
        .iter()
        .filter(|i| i.partition == field || i.sort == Some(field))
        .map(|index| {
            let present = std::iter::once(index.partition)
                .chain(index.sort)
                .flat_map(|i| inputs(record, i))
                .filter(|i| record.fields[*i].optional)
                .map(|i| {
                    let id = if receiver {
                        record.fields[i].ident.clone()
                    } else {
                        key::local(i)
                    };
                    if receiver {
                        quote!(self.#id.is_some())
                    } else {
                        quote!(#id.is_some())
                    }
                });
            quote!((true #(&& #present)*))
        });
    quote!((false #(|| #members)*))
}
pub fn encode(record: &Record, field: usize, body: TokenStream) -> TokenStream {
    if !conditional(record, field) {
        return body;
    }
    let member = membership(record, field, true);
    let validation = validate_sources(record, field, true);
    let fields: Vec<_> = inputs(record, field)
        .filter(|i| record.fields[*i].optional)
        .map(|i| &record.fields[i].ident)
        .collect();
    if fields.is_empty() {
        return quote!({ #validation if #member { #body } });
    }
    quote! {
        #validation
        if #member {
            if let (#(::core::option::Option::Some(#fields),)*) = (#(&self.#fields,)*) { #body }
        }
    }
}
pub fn decode(record: &Record, field: usize) -> TokenStream {
    let path = &record.crate_path;
    let item = format_ident!("item", span = Span::mixed_site());
    let member = membership(record, field, false);
    let name = &record.fields[field].name;
    let cx = key::component_context(record, field, None);
    let parse = key::decode(record, field);
    let validation = validate_sources(record, field, false);
    let checks = inputs(record, field).map(|i| {
        let local = key::local(i);
        let parsed = key::parsed(field, i);
        let mismatch = if record.fields[i].optional {
            quote!(#local.as_ref() != ::core::option::Option::Some(&#parsed))
        } else { quote!(#local != #parsed) };
        let cx = key::component_context(record, field, Some(i));
        quote!(if #mismatch { return ::core::result::Result::Err(#cx.error(#path::DecodeFault::KeyMismatch)); })
    });
    quote! {
        #validation
        if #member { #parse #(#checks)* }
        else if #item.contains_key(#name) { return ::core::result::Result::Err(#cx.error(#path::DecodeFault::IndexMismatch)); }
    }
}
/// Validate each present component even when a different input prevents membership.
fn validate_sources(record: &Record, field: usize, receiver: bool) -> TokenStream {
    let path = &record.crate_path;
    let fault = if receiver {
        quote!(#path::EncodeFault)
    } else {
        quote!(#path::DecodeFault)
    };
    let mut checks = Vec::new();
    let template = key::template(record, field).unwrap();
    for (position, part) in template.parts.iter().enumerate() {
        let Part::Component {
            field: i, codec, ..
        } = part
        else {
            continue;
        };
        let f = &record.fields[*i];
        let ty = &f.ty;
        let id = if receiver {
            f.ident.clone()
        } else {
            key::local(*i)
        };
        let value = if receiver {
            quote!(&self.#id)
        } else {
            quote!(&#id)
        };
        let codec_type = key::codec(*codec, f.ident.span(), path);
        let cx = key::component_context(record, field, Some(*i));
        let reserve = (*codec == crate::template::Codec::Text).then(|| key::boundary(&template.parts, position).1).flatten().map(|c| quote! {
                if component.contains(#c) { return ::core::result::Result::Err(#cx.error(#fault::KeyFormat(#path::KeyFormatFault::InvalidSeparator))); }
            });
        let check = quote! {
            let component = <#codec_type as #path::KeyComponentCodec<#ty>>::format(value).map_err(|fault| #cx.error(#fault::KeyFormat(fault)))?;
            #reserve
            let _ = component;
        };
        checks.push(if f.optional {
            quote!(if let ::core::option::Option::Some(value) = #value { #check })
        } else {
            quote!({ let value = #value; #check })
        });
    }
    quote!(#(#checks)*)
}
