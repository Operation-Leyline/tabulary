use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

use crate::{parse, validate::Record};

pub fn record(record: &Record) -> TokenStream {
    let Record {
        ident,
        vis,
        prefix,
        fields,
        partition,
        sort,
        allow_unknown,
    } = record;
    let attrs = format_ident!("{}Attrs", prefix);
    let key = format_ident!("{}Key", prefix);
    let primary = format_ident!("{}Primary", prefix);
    let indexes = format_ident!("{}Indexes", prefix);
    let item = format_ident!("item", span = Span::mixed_site());
    let context = format_ident!("cx", span = Span::mixed_site());
    let mut handles = Vec::new();
    let mut descriptors = Vec::new();
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    let mut key_fields = Vec::new();
    let mut key_args = Vec::new();
    let mut key_names = Vec::new();
    let mut key_values = Vec::new();
    let mut values = Vec::new();
    let mut clones = Vec::new();
    let mut checks = Vec::new();
    let mut key_writes = Vec::new();
    let mut key_reads = Vec::new();
    let order = std::iter::once(*partition).chain(*sort).chain(
        (0..fields.len()).filter(|i| i != partition && ::core::option::Option::Some(*i) != *sort),
    );
    for i in order {
        let f = &fields[i];
        let (id, ty, name, fv) = (&f.ident, &f.ty, &f.name, &f.vis);
        let local = format_ident!("__tabulary_field_{i}", span = Span::mixed_site());
        values.push(quote!(#id: #local));
        let handle = format_ident!(
            "{}",
            parse::rename(
                id.to_string().trim_start_matches("r#"),
                "SCREAMING_SNAKE_CASE"
            ),
            span = id.span()
        );
        let presence = if f.optional {
            quote!(::tabulary::Optional)
        } else {
            quote!(::tabulary::Required)
        };
        let access = if f.key_limit.is_some() {
            quote!(::tabulary::KeyOnly)
        } else {
            quote!(::tabulary::Mutable)
        };
        handles.push((i, quote_spanned! {id.span()=>
            #fv const #handle: ::tabulary::Attr<#ident, #ty, #presence, #access> = ::tabulary::__private::field(#name, #i);
        }));
        descriptors.push((i, quote!(#attrs::#handle.descriptor())));
        let cx = quote!(#context.at(::tabulary::PathSegment::Attribute(#name.into())));
        let (encode, decode) = if let ::core::option::Option::Some(max) = f.key_limit {
            key_fields.push(quote!(#fv #id: #ty));
            key_args.push(quote!(#id: #ty));
            key_names.push(id);
            key_values.push(quote!(#id: #local));
            clones.push(quote_spanned!(id.span()=> ::core::clone::Clone::clone(&self.#id)));
            checks.push(quote_spanned! {id.span()=>
                const _: () = ::core::assert!(::core::matches!(<<#ty as ::tabulary::Value>::Wire as ::tabulary::WireType>::DYNAMO_TYPE,
                    ::tabulary::DynamoType::S | ::tabulary::DynamoType::N | ::tabulary::DynamoType::B), "primary keys require S, N or B codecs");
            });
            (
                quote!(::core::option::Option::Some(::tabulary::__private::encode_key(&self.#id, &mut #cx, #max)?)),
                quote!(::tabulary::__private::decode_key(#item.get(#name), &mut #cx, #max)?),
            )
        } else if f.optional {
            (
                quote!(::tabulary::field::encode_optional::<#ty, ::tabulary::Native<#ty>>(&self.#id, &mut #cx)?),
                quote!(::tabulary::field::decode_optional::<#ty, ::tabulary::Native<#ty>>(#item.get(#name), &mut #cx)?),
            )
        } else {
            (
                quote!(::core::option::Option::Some(<#ty as ::tabulary::Value>::encode_value(&self.#id, &mut #cx)?)),
                quote!(::tabulary::field::decode_required::<#ty, ::tabulary::Native<#ty>>(#item.get(#name), &mut #cx)?),
            )
        };
        let decode = if let Some(default) = &f.default {
            let field_type = if f.optional {
                quote!(::core::option::Option<#ty>)
            } else {
                quote!(#ty)
            };
            quote_spanned! {id.span()=> match #item.get(#name) {
                ::core::option::Option::None => {
                    let default: fn() -> #field_type = #default;
                    default()
                },
                ::core::option::Option::Some(_) => #decode,
            }}
        } else {
            decode
        };
        let write = quote_spanned!(id.span()=> if let ::core::option::Option::Some(value) = #encode { #item.insert(#name.into(), value); });
        let read = quote_spanned!(id.span()=> let #local = #decode;);
        if f.key_limit.is_some() {
            key_writes.push(write.clone());
            key_reads.push(read.clone());
        }
        writes.push(write);
        reads.push(read);
    }
    handles.sort_by_key(|(i, _)| *i);
    descriptors.sort_by_key(|(i, _)| *i);
    let handles = handles.iter().map(|(_, tokens)| tokens);
    let descriptors = descriptors.iter().map(|(_, tokens)| tokens);
    let wire_names = fields.iter().map(|f| &f.name);
    let unknown_check = if *allow_unknown {
        quote!()
    } else {
        quote! {
            if let ::core::option::Option::Some(name) = #item.keys().filter(|name| ![#(#wire_names,)*].contains(&name.as_str())).min() {
                return ::core::result::Result::Err(#context.at(::tabulary::PathSegment::Attribute(name.clone())).error(::tabulary::DecodeFault::UnknownAttribute));
            }
        }
    };
    let sort = match sort {
        ::core::option::Option::Some(i) => quote!(::core::option::Option::Some(#i)),
        ::core::option::Option::None => quote!(::core::option::Option::None),
    };
    quote! {
        #(#checks)*
        #vis struct #attrs;
        impl #attrs { #(#handles)* }
        #vis struct #indexes;
        #[derive(::core::clone::Clone, ::core::marker::Copy)]
        #vis struct #primary;
        #[derive(::core::clone::Clone)]
        #vis struct #key { #(#key_fields,)* }
        impl #key {
            #vis fn new(#(#key_args),*) -> ::core::result::Result<Self, ::tabulary::EncodeError> {
                let key = Self { #(#key_names,)* };
                <Self as ::tabulary::RecordKey<#ident>>::encode_key(&key)?;
                ::core::result::Result::Ok(key)
            }
        }
        impl ::tabulary::RecordKey<#ident> for #key {
            fn encode_key(&self) -> ::core::result::Result<::tabulary::Item, ::tabulary::EncodeError> {
                let #context = ::tabulary::EncodeContext::new(::core::stringify!(#ident));
                let mut #item = ::tabulary::Item::new();
                #(#key_writes)*
                ::core::result::Result::Ok(#item)
            }
            fn decode_key(#item: &::tabulary::Item) -> ::core::result::Result<Self, ::tabulary::DecodeError> {
                let #context = ::tabulary::DecodeContext::new(::core::stringify!(#ident));
                #(#key_reads)*
                ::core::result::Result::Ok(Self { #(#key_values,)* })
            }
        }
        impl ::tabulary::Record for #ident {
            const SCHEMA: ::tabulary::RecordSchema = ::tabulary::__private::schema(
                ::core::stringify!(#ident), &[#(#descriptors,)*], ::tabulary::__private::keys(#partition, #sort));
        }
        impl #ident {
            #vis fn key(&self) -> ::core::result::Result<#key, ::tabulary::EncodeError> { #key::new(#(#clones),*) }
            #vis fn primary() -> #primary { #primary }
            #vis fn to_item(&self) -> ::core::result::Result<::tabulary::Item, ::tabulary::EncodeError> {
                let #context = ::tabulary::EncodeContext::new(::core::stringify!(#ident));
                let mut #item = ::tabulary::Item::new();
                #(#writes)*
                ::core::result::Result::Ok(#item)
            }
            #vis fn from_item(#item: &::tabulary::Item) -> ::core::result::Result<Self, ::tabulary::DecodeError> {
                let #context = ::tabulary::DecodeContext::new(::core::stringify!(#ident));
                #(#reads)*
                #unknown_check
                ::core::result::Result::Ok(Self { #(#values,)* })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn expansion_matches_fixture() {
        let input = syn::parse_str(include_str!("../tests/fixtures/record.rs")).unwrap();
        let actual = super::record(&crate::validate::record(input).unwrap());
        let expected: proc_macro2::TokenStream =
            include_str!("../tests/fixtures/record.expanded.tokens")
                .parse()
                .unwrap();
        assert_eq!(actual.to_string(), expected.to_string());
    }
}
