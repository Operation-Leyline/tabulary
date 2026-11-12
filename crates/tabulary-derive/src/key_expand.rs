//! Composite-key generation, kept separate from ordinary record expansion.
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

use crate::{
    template::{Codec, Part, Template},
    validate::Record,
};

pub fn local(field: usize) -> syn::Ident {
    format_ident!("__tabulary_field_{field}", span = Span::mixed_site())
}
fn parsed(key: usize, field: usize) -> syn::Ident {
    format_ident!("__tabulary_key_{key}_{field}", span = Span::mixed_site())
}
pub fn template(record: &Record, index: usize) -> Option<&Template> {
    record
        .templates
        .iter()
        .find(|(i, _)| *i == index)
        .map(|(_, t)| t)
}
pub fn components(template: &Template) -> impl Iterator<Item = usize> + '_ {
    template.parts.iter().filter_map(|p| match p {
        Part::Component { field, .. } => Some(*field),
        _ => None,
    })
}
pub fn inputs(record: &Record) -> Vec<usize> {
    let mut result = Vec::new();
    for index in std::iter::once(record.partition).chain(record.sort) {
        for field in template(record, index)
            .map(|t| components(t).collect())
            .unwrap_or_else(|| vec![index])
        {
            if !result.contains(&field) {
                result.push(field);
            }
        }
    }
    result
}
pub fn source(record: &Record, index: usize) -> bool {
    record
        .templates
        .iter()
        .any(|(_, t)| components(t).any(|i| i == index))
}
fn codec(codec: Codec, span: Span) -> TokenStream {
    let name = syn::Ident::new(
        match codec {
            Codec::Hex => "Hex",
            Codec::Dec => "Dec",
            Codec::Dec20 => "Dec20",
            Codec::Text => "Text",
        },
        span,
    );
    quote_spanned!(span=> ::tabulary::key_component::#name)
}
fn boundary(parts: &[Part], position: usize) -> (usize, Option<char>) {
    let mut tail = 0;
    for part in &parts[position + 1..] {
        match part {
            Part::Literal(s) => return (tail, s.chars().next()),
            // An overflowing tail cannot fit any service key; keep it impossible
            // to extract without panicking while expanding an enormous array type.
            Part::Component { width: Some(n), .. } => tail = tail.saturating_add(*n),
            _ => {}
        }
    }
    (tail, None)
}
fn component_context(record: &Record, key: usize, field: Option<usize>) -> TokenStream {
    let name = &record.fields[key].name;
    let context = format_ident!("cx", span = Span::mixed_site());
    let base = quote!(#context.at(::tabulary::PathSegment::Attribute(#name.into())));
    match field {
        Some(i) => {
            let name = record.fields[i].ident.to_string();
            let name = name.trim_start_matches("r#");
            quote!(#base.at(::tabulary::PathSegment::Component(#name)))
        }
        None => base,
    }
}
fn option<T: quote::ToTokens>(value: Option<T>) -> TokenStream {
    match value {
        Some(value) => quote!(::core::option::Option::Some(#value)),
        None => quote!(::core::option::Option::None),
    }
}

fn format(
    record: &Record,
    key: usize,
    parts: &[Part],
    receiver: bool,
    full: &[Part],
) -> TokenStream {
    let output = format_ident!("__tabulary_output", span = Span::mixed_site());
    let component = format_ident!("__tabulary_component", span = Span::mixed_site());
    let steps = parts.iter().enumerate().map(|(position, part)| match part {
        Part::Literal(s) => quote!(#output.push_str(#s);),
        Part::Component { field, codec: kind, .. } => {
            let f = &record.fields[*field]; let (id, ty) = (&f.ident, &f.ty);
            let value = if receiver { quote!(&self.#id) } else { quote!(&#id) };
            let cx = component_context(record, key, Some(*field)); let codec = codec(*kind, record.fields[*field].ident.span());
            let reserved = if *kind == Codec::Text { boundary(full, position).1 } else { None };
            let reserve = reserved.map(|c| quote! {
                if #component.contains(#c) { return ::core::result::Result::Err(#cx.error(::tabulary::EncodeFault::KeyFormat(::tabulary::KeyFormatFault::InvalidSeparator))); }
            });
            quote_spanned! {id.span()=>
                let #component = <#codec as ::tabulary::KeyComponentCodec<#ty>>::format(#value).map_err(|fault| #cx.error(::tabulary::EncodeFault::KeyFormat(fault)))?;
                #reserve
                #output.push_str(&#component);
            }
        }
    });
    quote! { let mut #output = ::std::string::String::new(); #(#steps)* }
}

pub fn encode(record: &Record, key: usize) -> TokenStream {
    let context = format_ident!("cx", span = Span::mixed_site());
    let item = format_ident!("item", span = Span::mixed_site());
    let parts = &template(record, key).unwrap().parts;
    let output = format_ident!("__tabulary_output", span = Span::mixed_site());
    let format = format(record, key, parts, true, parts);
    let (name, max) = (
        &record.fields[key].name,
        record.fields[key].key_limit.unwrap(),
    );
    quote! { { #format
        #item.insert(#name.into(), ::tabulary::__private::encode_key(&#output, &mut #context.at(::tabulary::PathSegment::Attribute(#name.into())), #max)?);
    } }
}

pub fn decode(record: &Record, key: usize) -> TokenStream {
    let context = format_ident!("cx", span = Span::mixed_site());
    let item = format_ident!("item", span = Span::mixed_site());
    let parts = &template(record, key).unwrap().parts;
    let (name, max) = (
        &record.fields[key].name,
        record.fields[key].key_limit.unwrap(),
    );
    let text = format_ident!("__tabulary_text_{key}", span = Span::mixed_site());
    let rest = format_ident!("__tabulary_rest_{key}", span = Span::mixed_site());
    let steps = parts.iter().enumerate().map(|(position, part)| {
        let nearest = parts[position..].iter().chain(parts[..position].iter().rev()).find_map(|p| match p { Part::Component { field, .. } => Some(*field), _ => None });
        let cx = component_context(record, key, nearest);
        match part {
            Part::Literal(s) => {
                let fault = if position == 0 { quote!(InvalidPrefix) } else { quote!(InvalidSeparator) };
                quote!(#rest = #rest.strip_prefix(#s).ok_or_else(|| #cx.error(::tabulary::DecodeFault::KeyFormat(::tabulary::KeyFormatFault::#fault)))?;)
            }
            Part::Component { field, codec: kind, width } => {
                let (tail, separator) = boundary(parts, position); let separator = option(separator); let width = option(*width);
                let local = parsed(key, *field); let codec = codec(*kind, record.fields[*field].ident.span()); let ty = &record.fields[*field].ty; let id = &record.fields[*field].ident;
                quote_spanned! {id.span()=>
                    let #local: #ty = {
                        let component = ::tabulary::key_component::take(&mut #rest, #width, #tail, #separator).map_err(|fault| #cx.error(::tabulary::DecodeFault::KeyFormat(fault)))?;
                        <#codec as ::tabulary::KeyComponentCodec<#ty>>::parse(component).map_err(|fault| #cx.error(::tabulary::DecodeFault::KeyFormat(fault)))?
                    };
                }
            }
        }
    });
    let cx = component_context(
        record,
        key,
        components(template(record, key).unwrap()).last(),
    );
    quote! {
        let #text: ::std::string::String = ::tabulary::__private::decode_key(#item.get(#name), &mut #context.at(::tabulary::PathSegment::Attribute(#name.into())), #max)?;
        let mut #rest = #text.as_str();
        #(#steps)*
        if !#rest.is_empty() { return ::core::result::Result::Err(#cx.error(::tabulary::DecodeFault::KeyFormat(::tabulary::KeyFormatFault::InvalidLength))); }
    }
}

pub fn checks(record: &Record, field: usize) -> TokenStream {
    let local = local(field);
    let checks = record.templates.iter().filter(|(_, t)| components(t).any(|i| i == field)).map(|(key, _)| {
        let parsed = parsed(*key, field); let cx = component_context(record, *key, Some(field));
        quote! { if #local != #parsed { return ::core::result::Result::Err(#cx.error(::tabulary::DecodeFault::KeyMismatch)); } }
    });
    quote!(#(#checks)*)
}

pub fn key_bindings(record: &Record) -> TokenStream {
    let bindings = inputs(record).into_iter().map(|field| {
        if record.fields[field].key_limit.is_some() { return checks(record, field); }
        let key = record.templates.iter().find(|(_, t)| components(t).any(|i| i == field)).unwrap().0;
        let parsed = parsed(key, field); let local = local(field);
        let checks = record.templates.iter().filter(|(i, t)| *i != key && components(t).any(|i| i == field)).map(|(other_key, _)| {
            let other = self::parsed(*other_key, field); let cx = component_context(record, *other_key, Some(field));
            quote! { if #parsed != #other { return ::core::result::Result::Err(#cx.error(::tabulary::DecodeFault::KeyMismatch)); } }
        });
        quote! { #(#checks)* let #local = #parsed; }
    });
    quote!(#(#bindings)*)
}

pub fn helpers(record: &Record) -> TokenStream {
    let context = format_ident!("cx", span = Span::mixed_site());
    let output = format_ident!("__tabulary_output", span = Span::mixed_site());
    let (ident, vis) = (&record.ident, &record.vis);
    let primary = format_ident!("{}Primary", record.prefix);
    let partition = template(record, record.partition).map(|t| {
        let args = components(t).map(|i| { let f = &record.fields[i]; let (id, ty) = (&f.ident, &f.ty); quote!(#id: #ty) });
        let format = format(record, record.partition, &t.parts, false, &t.parts);
        let name = &record.fields[record.partition].name;
        quote! { #vis fn partition(&self, #(#args),*) -> ::core::result::Result<::tabulary::key_component::KeyPrefix<#ident>, ::tabulary::EncodeError> {
            let #context = ::tabulary::EncodeContext::new(::core::stringify!(#ident)); #format
            ::tabulary::__private::encode_key(&#output, &mut #context.at(::tabulary::PathSegment::Attribute(#name.into())), 2048)?;
            ::core::result::Result::Ok(::tabulary::key_component::KeyPrefix::from_components(#output))
        } }
    });
    let mut implementations = Vec::new();
    if let Some((key, t)) = record
        .sort
        .and_then(|i| template(record, i).map(|t| (i, t)))
    {
        let positions: Vec<_> = t
            .parts
            .iter()
            .enumerate()
            .filter_map(|(p, part)| matches!(part, Part::Component { .. }).then_some(p))
            .collect();
        for end in positions.iter().skip(1) {
            let parts = &t.parts[..*end];
            let fields: Vec<_> = parts
                .iter()
                .filter_map(|p| match p {
                    Part::Component { field, .. } => Some(&record.fields[*field]),
                    _ => None,
                })
                .collect();
            let ids = fields.iter().map(|f| &f.ident);
            let types = fields.iter().map(|f| &f.ty);
            let format = format(record, key, parts, false, &t.parts);
            let name = &record.fields[key].name;
            implementations.push(quote! {
                impl ::tabulary::key_component::PrefixParts<#ident> for (#(#types,)*) {
                    fn format_prefix(self) -> ::core::result::Result<::tabulary::key_component::KeyPrefix<#ident>, ::tabulary::EncodeError> {
                        let (#(#ids,)*) = self;
                        let #context = ::tabulary::EncodeContext::new(::core::stringify!(#ident)); #format
                        ::tabulary::__private::encode_key(&#output, &mut #context.at(::tabulary::PathSegment::Attribute(#name.into())), 1024)?;
                        ::core::result::Result::Ok(::tabulary::key_component::KeyPrefix::from_components(#output))
                    }
                }
            });
        }
    }
    let prefix = (!implementations.is_empty()).then(|| quote! {
        #vis fn sort_prefix<P: ::tabulary::key_component::PrefixParts<#ident>>(&self, parts: P) -> ::core::result::Result<::tabulary::key_component::KeyPrefix<#ident>, ::tabulary::EncodeError> { parts.format_prefix() }
    });
    quote! { impl #primary { #partition #prefix } #(#implementations)* }
}
