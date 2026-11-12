use crate::variants_validate::Variants;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

pub fn variants(s: &Variants) -> TokenStream {
    let (id, vis, path, tag) = (&s.input.ident, &s.input.vis, &s.path, &s.tag);
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    let tag_value = format_ident!("tag", span = Span::mixed_site());
    let mut generics = s.input.generics.clone();
    if !generics.params.is_empty() {
        for f in s.variants.iter().flat_map(|v| &v.fields) {
            let (ty, codec) = (&f.ty, &f.codec);
            if crate::variants_validate::references_self(quote!(#ty), id) {
                continue;
            }
            generics
                .make_where_clause()
                .predicates
                .push(syn::parse_quote!(#codec: #path::Converter<#ty>));
        }
    }
    let (ig, tg, wc) = generics.split_for_impl();
    let mut encoders = Vec::new();
    let mut decoders = Vec::new();
    for v in &s.variants {
        let (variant, name) = (&v.id, &v.name);
        let mut writes = Vec::new();
        let mut reads = Vec::new();
        let mut bindings = Vec::new();
        let mut values = Vec::new();
        for (i, f) in v.fields.iter().enumerate() {
            let (field, wire, ty, codec) = (&f.id, &f.name, &f.ty, &f.codec);
            let local = format_ident!("__tabulary_payload_{i}", span = Span::mixed_site());
            let child = quote!(#cx.at(#path::PathSegment::Attribute(#wire.into())));
            bindings.push(quote!(#field: #local));
            values.push(quote!(#field: #local));
            let (encode, decode) = if f.optional {
                (
                    quote!(#path::field::encode_optional::<#ty, #codec>(#local, &mut #child)?),
                    quote!(#path::field::decode_optional::<#ty, #codec>(#item.get(#wire), &mut #child)?),
                )
            } else {
                (
                    quote!(::core::option::Option::Some(<#codec as #path::Converter<#ty>>::encode(#local, &mut #child)?)),
                    quote!(#path::field::decode_required::<#ty, #codec>(#item.get(#wire), &mut #child)?),
                )
            };
            let decode = if let Some(default) = &f.default {
                let full_ty = if f.optional {
                    quote!(::core::option::Option<#ty>)
                } else {
                    quote!(#ty)
                };
                quote!(match #item.get(#wire) { ::core::option::Option::None => { let default: fn() -> #full_ty = #default; default() }, ::core::option::Option::Some(_) => #decode })
            } else {
                decode
            };
            writes.push(quote!(if let ::core::option::Option::Some(value) = #encode { #item.insert(#wire.into(), value); }));
            reads.push(quote!(let #local = #decode;));
        }
        let pattern = if v.named {
            quote!(Self::#variant { #(#bindings,)* })
        } else {
            quote!(Self::#variant)
        };
        let value = if v.named {
            quote!(Self::#variant { #(#values,)* })
        } else {
            quote!(Self::#variant)
        };
        let active: Vec<_> = v.fields.iter().map(|f| &f.name).collect();
        let inactive = s.names.iter().filter(|n| !active.contains(n));
        encoders.push(quote!(#pattern => { #item.insert(#tag.into(), #path::__private::AttributeValue::S(#name.into())); #(#writes)* }));
        decoders.push(quote!(#name => {
            #(#reads)*
            if let ::core::option::Option::Some(name) = [#(#inactive,)*].iter().copied().find(|name: &&str| #item.contains_key(*name)) {
                return ::core::result::Result::Err(#cx.at(#path::PathSegment::Attribute(name.into())).error(#path::DecodeFault::InactiveVariantAttribute));
            }
            ::core::result::Result::Ok(#value)
        }));
    }
    let names = &s.names;
    let unknown = (!s.allow_unknown).then(|| quote! {
        if let ::core::option::Option::Some(name) = #item.keys().filter(|n| !<Self as #path::Variants>::ATTRIBUTES.contains(&n.as_str())).min() {
            return ::core::result::Result::Err(#cx.at(#path::PathSegment::Attribute(name.clone())).error(#path::DecodeFault::UnknownAttribute));
        }
    });
    let tag_id = format_ident!("{}Tag", s.prefix);
    let tags = format_ident!("{}Tags", s.prefix);
    let tag_variants = s.variants.iter().map(|v| &v.id);
    let constants = s.variants.iter().map(|v| {
        let name = format_ident!(
            "{}",
            crate::parse::rename(
                v.id.to_string().trim_start_matches("r#"),
                "SCREAMING_SNAKE_CASE"
            )
        );
        let variant = &v.id;
        quote!(#vis const #name: #tag_id = #tag_id::#variant;)
    });
    let tag_codec =
        crate::string_enum_expand::string_enum(&crate::string_enum_validate::StringEnum {
            ident: tag_id.clone(),
            generics: syn::Generics::default(),
            crate_path: path.clone(),
            variants: s
                .variants
                .iter()
                .map(|v| (v.id.clone(), v.name.clone()))
                .collect(),
        });
    let encode = if s.variants.is_empty() {
        quote!(match *self {})
    } else {
        quote! {
            let #cx = #cx.in_document(<Self as #path::Document>::NAME);
            let mut #item = #path::Item::new();
            match self { #(#encoders,)* }
            ::core::result::Result::Ok(#item)
        }
    };
    quote! {
        #[derive(::core::clone::Clone, ::core::marker::Copy, ::core::fmt::Debug, ::core::cmp::PartialEq, ::core::cmp::Eq)]
        #vis enum #tag_id { #(#tag_variants,)* }
        #vis struct #tags;
        #[allow(dead_code)]
        impl #tags { #(#constants)* }
        #tag_codec
        impl #ig #path::Variants for #id #tg #wc {
            type Tag = #tag_id;
            const TAG: &'static str = #tag;
            const ATTRIBUTES: &'static [&'static str] = &[#tag, #(#names,)*];
            fn decode_flattened(#item: &#path::Item, #cx: &mut #path::DecodeContext) -> ::core::result::Result<Self, #path::DecodeError> {
                let #cx = #cx.in_document(<Self as #path::Document>::NAME);
                let #tag_value = #path::field::decode_required::<::std::string::String, #path::Native<::std::string::String>>(
                    #item.get(#tag), &mut #cx.at(#path::PathSegment::Attribute(#tag.into())))?;
                match #tag_value.as_str() { #(#decoders,)* _ => ::core::result::Result::Err(#cx.at(#path::PathSegment::Attribute(#tag.into())).error(#path::DecodeFault::UnknownVariant)) }
            }
        }
        impl #ig #path::Document for #id #tg #wc {
            const NAME: &'static str = ::core::stringify!(#id);
            fn encode_map(&self, #cx: &mut #path::EncodeContext) -> ::core::result::Result<#path::Item, #path::EncodeError> {
                #encode
            }
            fn decode_map(#item: &#path::Item, #cx: &mut #path::DecodeContext) -> ::core::result::Result<Self, #path::DecodeError> {
                let value = <Self as #path::Variants>::decode_flattened(#item, #cx)?;
                let #cx = #cx.in_document(<Self as #path::Document>::NAME);
                #unknown
                ::core::result::Result::Ok(value)
            }
        }
        impl #ig #path::Value for #id #tg #wc {
            type Wire = #path::wire::Map;
            fn encode_value(&self, #cx: &mut #path::EncodeContext) -> ::core::result::Result<#path::__private::AttributeValue, #path::EncodeError> {
                let mut #cx = #cx.in_document(<Self as #path::Document>::NAME).enter_container()?;
                <Self as #path::Document>::encode_map(self, &mut #cx).map(#path::__private::AttributeValue::M)
            }
            fn decode_value(value: &#path::__private::AttributeValue, #cx: &mut #path::DecodeContext) -> ::core::result::Result<Self, #path::DecodeError> {
                let mut #cx = #cx.in_document(<Self as #path::Document>::NAME);
                let #path::__private::AttributeValue::M(#item) = value else { return ::core::result::Result::Err(#path::__private::wrong_type(value, #path::DynamoType::M, &#cx)); };
                #cx = #cx.enter_container()?;
                <Self as #path::Document>::decode_map(#item, &mut #cx)
            }
        }
    }
}
