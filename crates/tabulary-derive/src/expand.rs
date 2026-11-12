use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};

use crate::{key_expand, parse, validate::Record};

pub fn record(record: &Record) -> TokenStream {
    let Record {
        ident,
        generics: _,
        crate_path: path,
        vis,
        prefix,
        fields,
        partition,
        sort,
        ..
    } = record;
    let generics = crate::document_validate::bounds(record);
    let (ig, tg, wc) = generics.split_for_impl();
    let generic = !record.generics.params.is_empty();
    let mut marker_name = "__tabulary_marker".to_owned();
    while fields.iter().any(|field| field.ident == marker_name) {
        marker_name.push('_');
    }
    let marker = format_ident!("{}", marker_name, span = Span::mixed_site());
    let marker_decl =
        generic.then(|| quote!(#marker: ::core::marker::PhantomData<fn() -> #ident #tg>,));
    let marker_value = generic.then(|| quote!(#marker: ::core::marker::PhantomData,));
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
    let mut flatten_read = None;
    let mut key_fields = Vec::new();
    let mut key_args = Vec::new();
    let mut key_names = Vec::new();
    let mut key_values = Vec::new();
    let mut values = Vec::new();
    let mut clones = Vec::new();
    let mut checks = Vec::new();
    let mut key_writes = Vec::new();
    let mut key_reads = Vec::new();
    let mut source_checks = Vec::new();
    let mut common_reads = Vec::new();
    let mut sparse_reads = Vec::new();
    let inputs = key_expand::inputs(record);
    for &i in &inputs {
        let f = &fields[i];
        let (id, ty, fv) = (&f.ident, &f.ty, &f.vis);
        let local = key_expand::local(i);
        key_fields.push(quote!(#fv #id: #ty));
        key_args.push(quote!(#id: #ty));
        key_names.push(id);
        key_values.push(quote!(#id: #local));
        clones.push(quote_spanned!(id.span()=> ::core::clone::Clone::clone(&self.#id)));
    }
    let mut order: Vec<_> = std::iter::once(*partition).chain(*sort).collect();
    for i in inputs.iter().copied().chain(0..fields.len()) {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    for i in order {
        let f = &fields[i];
        let (id, ty, name, fv) = (&f.ident, &f.ty, &f.name, &f.vis);
        let local = format_ident!("__tabulary_field_{i}", span = Span::mixed_site());
        let synthetic = key_expand::template(record, i).is_some();
        if !synthetic {
            values.push(quote!(#id: #local));
        }
        let handle = format_ident!(
            "{}",
            parse::rename(
                id.to_string().trim_start_matches("r#"),
                "SCREAMING_SNAKE_CASE"
            ),
            span = id.span()
        );
        let presence = if f.optional {
            quote!(#path::Optional)
        } else {
            quote!(#path::Required)
        };
        if f.flatten {
            let (write, read, tag) = crate::variants_flatten::field(record, i);
            handles.push((i, quote_spanned!(id.span()=> #fv const #handle: #path::Attr<#ident #tg, #tag, #path::Required, #path::DerivedOnly> = #path::__private::field(<#ty as #path::Variants>::TAG, #i);)));
            descriptors.push((i, quote!(<#attrs #tg>::#handle.descriptor())));
            writes.push(write);
            flatten_read = Some(read);
            continue;
        }
        let access = key_expand::access(record, i);
        handles.push((i, quote_spanned! {id.span()=>
            #fv const #handle: #path::Attr<#ident #tg, #ty, #presence, #access> = #path::__private::field(#name, #i);
        }));
        descriptors.push((i, quote!(<#attrs #tg>::#handle.descriptor())));
        if synthetic {
            let write = key_expand::encode(record, i);
            let read = key_expand::decode(record, i);
            writes.push(write.clone());
            if key_expand::primary(record, i) {
                key_writes.push(write);
            }
            if crate::sparse_expand::conditional(record, i) {
                sparse_reads.push(crate::sparse_expand::decode(record, i));
            } else {
                reads.push(read.clone());
            }
            if key_expand::primary(record, i) {
                key_reads.push(read);
            }
            continue;
        }
        let cx = quote!(#context.at(#path::PathSegment::Attribute(#name.into())));
        let (encode, decode) = if let ::core::option::Option::Some(max) = f
            .key_limit
            .into_iter()
            .chain(key_expand::index_limit(record, i))
            .min()
        {
            let key_message = if key_expand::primary(record, i) {
                "primary keys require S, N or B codecs"
            } else {
                "index keys require S, N or B codecs"
            };
            checks.push(quote_spanned! {id.span()=>
                const _: () = ::core::assert!(::core::matches!(<<#ty as #path::Value>::Wire as #path::WireType>::DYNAMO_TYPE,
                    #path::DynamoType::S | #path::DynamoType::N | #path::DynamoType::B), #key_message);
            });
            let (encode, decode) = (
                quote!(::core::option::Option::Some(#path::__private::encode_key(&self.#id, &mut #cx, #max)?)),
                quote!(#path::__private::decode_key(#item.get(#name), &mut #cx, #max)?),
            );
            if f.optional {
                (
                    quote!(match &self.#id {
                        ::core::option::Option::Some(value) => ::core::option::Option::Some(#path::__private::encode_key(value, &mut #cx, #max)?),
                        ::core::option::Option::None => ::core::option::Option::None,
                    }),
                    quote!(match #item.get(#name) {
                        ::core::option::Option::Some(_) => ::core::option::Option::Some(#decode),
                        ::core::option::Option::None => ::core::option::Option::None,
                    }),
                )
            } else {
                (encode, decode)
            }
        } else if f.optional {
            (
                quote!(#path::field::encode_optional::<#ty, #path::Native<#ty>>(&self.#id, &mut #cx)?),
                quote!(#path::field::decode_optional::<#ty, #path::Native<#ty>>(#item.get(#name), &mut #cx)?),
            )
        } else {
            (
                quote!(::core::option::Option::Some(<#ty as #path::Value>::encode_value(&self.#id, &mut #cx)?)),
                quote!(#path::field::decode_required::<#ty, #path::Native<#ty>>(#item.get(#name), &mut #cx)?),
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
        if inputs.contains(&i) {
            reads.push(read);
        } else {
            common_reads.push(read);
        }
        if key_expand::source(record, i) || key_expand::index_source(record, i) {
            source_checks.push(key_expand::checks(record, i));
        }
    }
    reads.extend(common_reads);
    reads.extend(sparse_reads);
    reads.extend(source_checks);
    handles.sort_by_key(|(i, _)| *i);
    descriptors.sort_by_key(|(i, _)| *i);
    let handles = handles.iter().map(|(_, tokens)| tokens);
    let descriptors = descriptors.iter().map(|(_, tokens)| tokens);
    reads.extend(flatten_read);
    let unknown_check = crate::document_expand::unknown_check(record);
    let sort = match sort {
        ::core::option::Option::Some(i) => quote!(::core::option::Option::Some(#i)),
        ::core::option::Option::None => quote!(::core::option::Option::None),
    };
    let (checks, key_check) = if generic {
        // Associated constants may refer to generic field types; evaluate before key use.
        let assertions = fields.iter().enumerate().filter(|(i, f)| f.key_limit.is_some() || key_expand::index_limit(record, *i).is_some()).map(|(i, f)| {
            let key_message = if key_expand::primary(record, i) { "primary keys require S, N or B codecs" } else { "index keys require S, N or B codecs" };
            let ty = &f.ty;
            quote_spanned!(f.ident.span()=> ::core::assert!(::core::matches!(<<#ty as #path::Value>::Wire as #path::WireType>::DYNAMO_TYPE,
                #path::DynamoType::S | #path::DynamoType::N | #path::DynamoType::B), #key_message);)
        });
        (
            quote!(),
            quote!(const __TABULARY_KEYS: () = { #(#assertions)* };),
        )
    } else {
        (quote!(#(#checks)*), quote!())
    };
    let check_keys = generic.then(|| quote!(let () = <#ident #tg>::__TABULARY_KEYS;));
    if let Some(check) = &check_keys {
        writes.insert(0, check.clone());
        reads.insert(0, check.clone());
    }
    let document =
        crate::document_expand::implementation(record, &writes, &reads, &values, unknown_check);
    let peers = if generic {
        quote! {
            #vis struct #attrs #generics (::core::marker::PhantomData<fn() -> #ident #tg>) #wc;
            #vis struct #indexes #generics (::core::marker::PhantomData<fn() -> #ident #tg>) #wc;
            #vis struct #primary #generics (::core::marker::PhantomData<fn() -> #ident #tg>) #wc;
            impl #ig ::core::marker::Copy for #primary #tg #wc {}
            impl #ig ::core::clone::Clone for #primary #tg #wc { fn clone(&self) -> Self { *self } }
        }
    } else {
        quote!(#vis struct #attrs; #vis struct #indexes; #[derive(::core::clone::Clone, ::core::marker::Copy)] #vis struct #primary;)
    };
    let primary_value = if generic {
        quote!(#primary(::core::marker::PhantomData))
    } else {
        quote!(#primary)
    };
    let key_clone = if generic {
        let fields = inputs.iter().map(|i| &fields[*i].ident);
        quote!(impl #ig ::core::clone::Clone for #key #tg #wc {
            fn clone(&self) -> Self { Self { #(#fields: ::core::clone::Clone::clone(&self.#fields),)* #marker_value } }
        })
    } else {
        quote!()
    };
    let clone_derive = (!generic).then(|| quote!(#[derive(::core::clone::Clone)]));
    let key_bindings = key_expand::key_bindings(record);
    let index_peers = crate::index_expand::expand(record);
    let index_schema = (!record.indexes.is_empty()).then(|| {
        let definitions = record
            .indexes
            .iter()
            .map(|i| crate::index_expand::definition(record, i));
        quote!(.with_indexes(&[#(#definitions),*]))
    });
    let helpers = key_expand::helpers(record);
    let primary_schema = crate::query_expand::primary(record);
    quote! {
        #checks
        #peers
        #primary_schema
        #helpers
        #index_peers
        impl #ig #attrs #tg #wc { #(#handles)* }
        #clone_derive
        #vis struct #key #generics #wc { #(#key_fields,)* #marker_decl }
        #key_clone
        impl #ig #key #tg #wc {
            #vis fn new(#(#key_args),*) -> ::core::result::Result<Self, #path::EncodeError> {
                let key = Self { #(#key_names,)* #marker_value };
                <Self as #path::RecordKey<#ident #tg>>::encode_key(&key)?;
                ::core::result::Result::Ok(key)
            }
        }
        impl #ig #path::RecordKey<#ident #tg> for #key #tg #wc {
            fn encode_key(&self) -> ::core::result::Result<#path::Item, #path::EncodeError> {
                #check_keys
                let #context = #path::EncodeContext::new(::core::stringify!(#ident));
                let mut #item = #path::Item::new();
                #(#key_writes)*
                ::core::result::Result::Ok(#item)
            }
            fn decode_key(#item: &#path::Item) -> ::core::result::Result<Self, #path::DecodeError> {
                #check_keys
                let #context = #path::DecodeContext::new(::core::stringify!(#ident));
                #(#key_reads)*
                #key_bindings
                ::core::result::Result::Ok(Self { #(#key_values,)* #marker_value })
            }
        }
        impl #ig #path::Record for #ident #tg #wc {
            const SCHEMA: #path::RecordSchema = #path::__private::schema(
                ::core::stringify!(#ident), &[#(#descriptors,)*], #path::__private::keys(#partition, #sort)) #index_schema;
        }
        impl #ig #ident #tg #wc {
            #key_check
            #vis fn key(&self) -> ::core::result::Result<#key #tg, #path::EncodeError> { <#key #tg>::new(#(#clones),*) }
            #vis fn primary() -> #primary #tg { #primary_value }
            #vis fn to_item(&self) -> ::core::result::Result<#path::Item, #path::EncodeError> {
                <Self as #path::Document>::encode_map(self, &mut #path::EncodeContext::new(::core::stringify!(#ident)))
            }
            #vis fn from_item(#item: &#path::Item) -> ::core::result::Result<Self, #path::DecodeError> {
                <Self as #path::Document>::decode_map(#item, &mut #path::DecodeContext::new(::core::stringify!(#ident)))
            }
        }
        #document
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
