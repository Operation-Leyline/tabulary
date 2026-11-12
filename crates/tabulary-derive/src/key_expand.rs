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
pub fn parsed(key: usize, field: usize) -> syn::Ident {
    format_ident!("__tabulary_key_{key}_{field}", span = Span::mixed_site())
}
pub fn template(record: &Record, index: usize) -> Option<&Template> {
    record
        .templates
        .iter()
        .find_map(|(i, t)| (*i == index).then_some(t))
}
pub fn components(parts: &[Part]) -> impl Iterator<Item = usize> + '_ {
    parts.iter().filter_map(|p| match p {
        Part::Component { field, .. } => Some(*field),
        _ => None,
    })
}
pub fn inputs(record: &Record) -> Vec<usize> {
    let mut result = Vec::new();
    for index in std::iter::once(record.partition).chain(record.sort) {
        for field in template(record, index)
            .map(|t| components(&t.parts).collect())
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
        .any(|(key, t)| primary(record, *key) && components(&t.parts).any(|i| i == index))
}
pub fn primary(record: &Record, index: usize) -> bool {
    index == record.partition || Some(index) == record.sort
}
pub fn index_limit(record: &Record, index: usize) -> Option<usize> {
    record
        .indexes
        .iter()
        .flat_map(|i| [(i.partition, 2048), (i.sort.unwrap_or(usize::MAX), 1024)])
        .filter_map(|(i, max)| (i == index).then_some(max))
        .min()
}
pub fn index_source(record: &Record, index: usize) -> bool {
    record
        .templates
        .iter()
        .any(|(key, t)| !primary(record, *key) && components(&t.parts).any(|i| i == index))
}
pub fn access(record: &Record, index: usize) -> TokenStream {
    let path = &record.crate_path;
    if template(record, index).is_some() {
        quote!(#path::DerivedOnly)
    } else if record.version == Some(index) {
        quote!(#path::VersionOnly)
    } else if record.fields[index].key_limit.is_some() || source(record, index) {
        quote!(#path::KeyOnly)
    } else if index_source(record, index) {
        quote!(#path::IndexSourceOnly)
    } else {
        quote!(#path::Mutable)
    }
}
pub fn bounds(record: &Record, generics: &mut syn::Generics) {
    let path = &record.crate_path;
    for (_, template) in &record.templates {
        for part in &template.parts {
            if let Part::Component {
                field, codec: kind, ..
            } = part
            {
                let field = &record.fields[*field];
                let ty = &field.ty;
                let codec = codec(*kind, field.ident.span(), path);
                generics.make_where_clause().predicates.push(syn::parse_quote_spanned!(field.ident.span()=> #codec: #path::KeyComponentCodec<#ty>));
                generics.make_where_clause().predicates.push(
                    syn::parse_quote_spanned!(field.ident.span()=> #ty: ::core::cmp::PartialEq),
                );
            }
        }
    }
}
pub fn codec(codec: Codec, span: Span, path: &syn::Path) -> TokenStream {
    let name = syn::Ident::new(
        match codec {
            Codec::Hex => "Hex",
            Codec::Dec => "Dec",
            Codec::Dec20 => "Dec20",
            Codec::Text => "Text",
        },
        span,
    );
    quote_spanned!(span=> #path::key_component::#name)
}
pub fn boundary(parts: &[Part], position: usize) -> (usize, Option<char>) {
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
pub fn component_context(record: &Record, key: usize, field: Option<usize>) -> TokenStream {
    let path = &record.crate_path;
    let name = &record.fields[key].name;
    let context = format_ident!("cx", span = Span::mixed_site());
    let base = quote!(#context.at(#path::PathSegment::Attribute(#name.into())));
    match field {
        Some(i) => {
            let name = record.fields[i].ident.to_string();
            let name = name.trim_start_matches("r#");
            quote!(#base.at(#path::PathSegment::Component(#name)))
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
    let path = &record.crate_path;
    let output = format_ident!("__tabulary_output", span = Span::mixed_site());
    let component = format_ident!("__tabulary_component", span = Span::mixed_site());
    let steps = parts.iter().enumerate().map(|(position, part)| match part {
        Part::Literal(s) => quote!(#output.push_str(#s);),
        Part::Component { field, codec: kind, .. } => {
            let f = &record.fields[*field]; let (id, ty) = (&f.ident, &f.ty);
            let value = if receiver && !f.optional { quote!(&self.#id) } else { quote!(&#id) };
            let cx = component_context(record, key, Some(*field)); let codec = codec(*kind, record.fields[*field].ident.span(), &record.crate_path);
            let reserved = if *kind == Codec::Text { boundary(full, position).1 } else { None };
            let reserve = reserved.map(|c| quote! {
                if #component.contains(#c) { return ::core::result::Result::Err(#cx.error(#path::EncodeFault::KeyFormat(#path::KeyFormatFault::InvalidSeparator))); }
            });
            quote_spanned! {id.span()=>
                let #component = <#codec as #path::KeyComponentCodec<#ty>>::format(#value).map_err(|fault| #cx.error(#path::EncodeFault::KeyFormat(fault)))?;
                #reserve
                #output.push_str(&#component);
            }
        }
    });
    quote! { let mut #output = ::std::string::String::new(); #(#steps)* }
}

pub fn encode(record: &Record, key: usize) -> TokenStream {
    crate::sparse_expand::encode(record, key, encode_body(record, key))
}
pub fn encode_body(record: &Record, key: usize) -> TokenStream {
    let path = &record.crate_path;
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
        #item.insert(#name.into(), #path::__private::encode_key(&#output, &mut #context.at(#path::PathSegment::Attribute(#name.into())), #max)?);
    } }
}

pub fn decode(record: &Record, key: usize) -> TokenStream {
    let path = &record.crate_path;
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
                quote!(#rest = #rest.strip_prefix(#s).ok_or_else(|| #cx.error(#path::DecodeFault::KeyFormat(#path::KeyFormatFault::#fault)))?;)
            }
            Part::Component { field, codec: kind, width } => {
                let (tail, separator) = boundary(parts, position); let separator = option(separator); let width = option(*width);
                let local = parsed(key, *field); let codec = codec(*kind, record.fields[*field].ident.span(), &record.crate_path); let ty = &record.fields[*field].ty; let id = &record.fields[*field].ident;
                quote_spanned! {id.span()=>
                    let #local: #ty = {
                        let component = #path::key_component::take(&mut #rest, #width, #tail, #separator).map_err(|fault| #cx.error(#path::DecodeFault::KeyFormat(fault)))?;
                        <#codec as #path::KeyComponentCodec<#ty>>::parse(component).map_err(|fault| #cx.error(#path::DecodeFault::KeyFormat(fault)))?
                    };
                }
            }
        }
    });
    let cx = component_context(
        record,
        key,
        components(&template(record, key).unwrap().parts).last(),
    );
    quote! {
        let #text: ::std::string::String = #path::__private::decode_key(#item.get(#name), &mut #context.at(#path::PathSegment::Attribute(#name.into())), #max)?;
        let mut #rest = #text.as_str();
        #(#steps)*
        if !#rest.is_empty() { return ::core::result::Result::Err(#cx.error(#path::DecodeFault::KeyFormat(#path::KeyFormatFault::InvalidLength))); }
    }
}

pub fn checks(record: &Record, field: usize) -> TokenStream {
    checks_for(record, field, false)
}
fn checks_for(record: &Record, field: usize, primary_only: bool) -> TokenStream {
    let path = &record.crate_path;
    let local = local(field);
    let checks = record.templates.iter().filter(|(key, t)| !crate::sparse_expand::conditional(record, *key) && (!primary_only || primary(record, *key)) && components(&t.parts).any(|i| i == field)).map(|(key, _)| {
        let parsed = parsed(*key, field); let cx = component_context(record, *key, Some(field));
        quote! { if #local != #parsed { return ::core::result::Result::Err(#cx.error(#path::DecodeFault::KeyMismatch)); } }
    });
    quote!(#(#checks)*)
}

pub fn key_bindings(record: &Record) -> TokenStream {
    let path = &record.crate_path;
    let bindings = inputs(record).into_iter().map(|field| {
        if primary(record, field) { return checks_for(record, field, true); }
        let key = record.templates.iter().find(|(key, t)| primary(record, *key) && components(&t.parts).any(|i| i == field)).unwrap().0;
        let parsed = parsed(key, field); let local = local(field);
        let checks = record.templates.iter().filter(|(i, t)| primary(record, *i) && *i != key && components(&t.parts).any(|i| i == field)).map(|(other_key, _)| {
            let other = self::parsed(*other_key, field); let cx = component_context(record, *other_key, Some(field));
            quote! { if #parsed != #other { return ::core::result::Result::Err(#cx.error(#path::DecodeFault::KeyMismatch)); } }
        });
        quote! { #(#checks)* let #local = #parsed; }
    });
    quote!(#(#bindings)*)
}

pub fn helpers(record: &Record) -> TokenStream {
    helpers_for(record, record.partition, record.sort, &record.prefix)
}
pub fn helpers_for(
    record: &Record,
    partition_key: usize,
    sort_key: Option<usize>,
    prefix: &syn::Ident,
) -> TokenStream {
    let path = &record.crate_path;
    let context = format_ident!("cx", span = Span::mixed_site());
    let output = format_ident!("__tabulary_output", span = Span::mixed_site());
    let (ident, vis) = (&record.ident, &record.vis);
    let primary = if prefix == &record.prefix {
        format_ident!("{}Primary", prefix)
    } else {
        prefix.clone()
    };
    let generics = crate::document_validate::bounds(record);
    let (ig, tg, wc) = generics.split_for_impl();
    let partition_value = format_ident!("{}Partition", prefix);
    let sort_value = format_ident!("{}SortPrefix", prefix);
    let prefix_parts = format_ident!("{}PrefixParts", prefix);
    let carrier = |name| {
        quote! {
            #vis struct #name #generics #wc { value: ::std::string::String, marker: ::core::marker::PhantomData<fn() -> #ident #tg> }
            impl #ig #name #tg #wc { #vis fn as_str(&self) -> &str { &self.value } }
            impl #ig ::core::fmt::Debug for #name #tg #wc {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result { f.write_str(::core::stringify!(#name)) }
            }
        }
    };
    let partition_carrier = template(record, partition_key).map(|_| carrier(&partition_value));
    let partition = template(record, partition_key).map(|t| {
        let ids: Vec<_> = components(&t.parts).map(|i| &record.fields[i].ident).collect();
        let args: Vec<_> = components(&t.parts).map(|i| { let f = &record.fields[i]; let (id, ty) = (&f.ident, &f.ty); quote!(#id: #ty) }).collect();
        let format = format(record, partition_key, &t.parts, false, &t.parts);
        let name = &record.fields[partition_key].name;
        let max = record.fields[partition_key].key_limit.unwrap();
        quote! {
          #vis fn partition(&self, #(#args),*) -> #path::KeyCondition<#ident #tg, Self> {
            let value = self.partition_value(#(#ids),*).map(|value| value.value);
            #path::__private::partition::<#ident #tg, Self, ::std::string::String, #path::Native<::std::string::String>>(value)
          }
          #vis fn partition_value(&self, #(#args),*) -> ::core::result::Result<#partition_value #tg, #path::EncodeError> {
            let #context = #path::EncodeContext::new(::core::stringify!(#ident)); #format
            #path::__private::encode_key(&#output, &mut #context.at(#path::PathSegment::Attribute(#name.into())), #max)?;
            ::core::result::Result::Ok(#partition_value { value: #output, marker: ::core::marker::PhantomData })
        } }
    }).unwrap_or_else(|| {
        let ty = &record.fields[partition_key].ty;
        let codec = record.fields[partition_key].codec(path);
        quote! { #vis fn partition(&self, value: #ty) -> #path::KeyCondition<#ident #tg, Self> where #ty: 'static {
            #path::__private::partition::<#ident #tg, Self, #ty, #codec>(::core::result::Result::Ok(value))
        } }
    });
    let mut implementations = Vec::new();
    if let Some((key, t)) = sort_key.and_then(|i| template(record, i).map(|t| (i, t))) {
        for (end, _) in t
            .parts
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p, Part::Component { .. }))
            .skip(1)
        {
            let parts = &t.parts[..end];
            // A variable endpoint needs its following literal. A fixed tail
            // cannot delimit a query prefix until that tail is supplied too.
            if parts
                .iter()
                .rev()
                .take_while(|p| !matches!(p, Part::Literal(_)))
                .any(|p| matches!(p, Part::Component { width: None, .. }))
            {
                continue;
            }
            let fields: Vec<_> = components(parts).map(|i| &record.fields[i]).collect();
            let ids = fields.iter().map(|f| &f.ident);
            let types = fields.iter().map(|f| &f.ty);
            let format = format(record, key, parts, false, &t.parts);
            let name = &record.fields[key].name;
            implementations.push(quote! {
                impl #ig #prefix_parts #tg for (#(#types,)*) #wc {
                    fn format_prefix(self) -> ::core::result::Result<#sort_value #tg, #path::EncodeError> {
                        let (#(#ids,)*) = self;
                        let #context = #path::EncodeContext::new(::core::stringify!(#ident)); #format
                        #path::__private::encode_key(&#output, &mut #context.at(#path::PathSegment::Attribute(#name.into())), 1024)?;
                        ::core::result::Result::Ok(#sort_value { value: #output, marker: ::core::marker::PhantomData })
                    }
                }
            });
        }
    }
    let sort_carrier = (!implementations.is_empty()).then(|| {
        let carrier = carrier(&sort_value);
        let codec = format_ident!("{}SortCodec", prefix);
        let types = components(&template(record, sort_key.unwrap()).unwrap().parts).map(|i| &record.fields[i].ty);
        quote! { #carrier #[doc(hidden)] #vis trait #prefix_parts #generics #wc {
            fn format_prefix(self) -> ::core::result::Result<#sort_value #tg, #path::EncodeError>;
        }
        impl #ig #path::PrefixCodec<(#(#types,)*)> for #codec #tg #wc {
            type Prefix = #sort_value #tg;
            fn encode_prefix(value: &Self::Prefix, cx: &mut #path::EncodeContext) -> ::core::result::Result<#path::__private::AttributeValue, #path::EncodeError> {
                #path::__private::encode_key(&value.value, cx, 1024)
            }
        } }
    });
    let prefix = (!implementations.is_empty()).then(|| quote! {
        #vis fn sort_prefix<P: #prefix_parts #tg>(&self, parts: P) -> ::core::result::Result<#sort_value #tg, #path::EncodeError> { parts.format_prefix() }
    });
    quote! { #partition_carrier #sort_carrier impl #ig #primary #tg #wc { #partition #prefix } #(#implementations)* }
}

/// Logical tuple codec for an index template, using the same formatter/parser.
pub fn template_codec(
    record: &Record,
    key: usize,
    codec: &syn::Ident,
) -> (TokenStream, TokenStream) {
    let path = &record.crate_path;
    let (owner, vis) = (&record.ident, &record.vis);
    let generics = crate::document_validate::bounds(record);
    let (ig, tg, wc) = generics.split_for_impl();
    let parts = &template(record, key).unwrap().parts;
    let fields: Vec<_> = components(parts).collect();
    let ids = fields.iter().map(|i| &record.fields[*i].ident);
    let types = fields.iter().map(|i| &record.fields[*i].ty);
    let logical = quote!((#(#types,)*));
    let parsed = fields.iter().map(|i| parsed(key, *i));
    let format = format(record, key, parts, false, parts);
    let decode = decode(record, key);
    let (name, max) = (
        &record.fields[key].name,
        record.fields[key].key_limit.unwrap(),
    );
    let cx = format_ident!("cx", span = Span::mixed_site());
    let item = format_ident!("item", span = Span::mixed_site());
    let output = format_ident!("__tabulary_output", span = Span::mixed_site());
    let unordered = parts.iter().enumerate().find_map(|(position, part)| {
        let Part::Component {
            field,
            codec,
            width,
        } = part
        else {
            return None;
        };
        let preserves_order = match codec {
            Codec::Dec20 => true,
            Codec::Hex => width.is_some(),
            // A suffix can reverse the order when one text value prefixes another.
            Codec::Text => position + 1 == parts.len(),
            Codec::Dec => false,
        };
        (!preserves_order).then_some(*field)
    });
    let ordered = unordered.is_none().then(|| {
        quote! {
            impl #ig #path::OrderedKeyCodec<#logical> for #codec #tg #wc {}
        }
    });
    // Point missing-capability errors at the component, retaining macro hygiene.
    let codec_span = unordered.map_or_else(Span::mixed_site, |field| {
        Span::mixed_site().located_at(record.fields[field].ident.span())
    });
    let mut declaration_name = codec.clone();
    declaration_name.set_span(codec_span);
    let codec_declaration = quote_spanned!(codec_span=>
        #vis struct #declaration_name #generics (::core::marker::PhantomData<fn() -> #owner #tg>) #wc;
    );
    let declaration = quote! {
        #codec_declaration
        #ordered
        impl #ig #path::Converter<#logical> for #codec #tg #wc {
            type Wire = #path::wire::String;
            fn encode(value: &#logical, #cx: &mut #path::EncodeContext) -> ::core::result::Result<#path::__private::AttributeValue, #path::EncodeError> {
                let (#(#ids,)*) = value; #format
                #path::__private::encode_key(&#output, #cx, #max)
            }
            fn decode(value: &#path::__private::AttributeValue, #cx: &mut #path::DecodeContext) -> ::core::result::Result<#logical, #path::DecodeError> {
                let #item = #path::Item::from([(#name.into(), value.clone())]); #decode
                ::core::result::Result::Ok((#(#parsed,)*))
            }
        }
    };
    (logical, declaration)
}
