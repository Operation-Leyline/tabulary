use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

use crate::{document_validate::bounds, parse, validate::Record};

pub fn document(schema: &Record) -> TokenStream {
    let (ident, vis, path) = (&schema.ident, &schema.vis, &schema.crate_path);
    let generics = bounds(schema);
    let (ig, tg, wc) = generics.split_for_impl();
    let attrs = format_ident!("{}Attrs", schema.prefix);
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    let mut values = Vec::new();
    let mut handles = Vec::new();
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    for (i, field) in schema.fields.iter().enumerate() {
        let (id, ty, name, fv) = (&field.ident, &field.ty, &field.name, &field.vis);
        let codec = field.codec(path);
        let local = format_ident!("__tabulary_field_{i}", span = Span::mixed_site());
        let handle = format_ident!(
            "{}",
            parse::rename(
                id.to_string().trim_start_matches("r#"),
                "SCREAMING_SNAKE_CASE"
            )
        );
        let child = quote!(#cx.at(#path::PathSegment::Attribute(#name.into())));
        if field.flatten {
            let (write, read, tag) = crate::variants_flatten::field(schema, i);
            writes.push(write);
            reads.push(read);
            values.push(quote!(#id: #local));
            handles.push(quote_spanned!(id.span()=> #fv const #handle: #path::Attr<#ident #tg, #tag, #path::Required, #path::DerivedOnly> = #path::__private::field(<#ty as #path::Variants>::TAG, #i);));
            continue;
        }
        let (presence, encode, decode) = if field.optional {
            (
                quote!(#path::Optional),
                quote!(#path::field::encode_optional::<#ty, #codec>(&self.#id, &mut #child)?),
                quote!(#path::field::decode_optional::<#ty, #codec>(#item.get(#name), &mut #child)?),
            )
        } else {
            (
                quote!(#path::Required),
                quote!(::core::option::Option::Some(<#codec as #path::Converter<#ty>>::encode(&self.#id, &mut #child)?)),
                quote!(#path::field::decode_required::<#ty, #codec>(#item.get(#name), &mut #child)?),
            )
        };
        let decode = if let Some(default) = &field.default {
            let ty = if field.optional {
                quote!(::core::option::Option<#ty>)
            } else {
                quote!(#ty)
            };
            quote!(match #item.get(#name) {
                ::core::option::Option::None => { let default: fn() -> #ty = #default; default() },
                ::core::option::Option::Some(_) => #decode,
            })
        } else {
            decode
        };
        writes.push(quote_spanned!(id.span()=> if let ::core::option::Option::Some(value) = #encode { #item.insert(#name.into(), value); }));
        reads.push(quote_spanned!(id.span()=> let #local = #decode;));
        values.push(quote!(#id: #local));
        handles.push(quote_spanned!(id.span()=> #fv const #handle: #path::Attr<#ident #tg, #ty, #presence, #path::Mutable, #codec> = #path::__private::field(#name, #i);));
    }
    let unknown = unknown_check(schema);
    // Common fields precede the discriminator even if flatten was declared first.
    if let Some(i) = schema.fields.iter().position(|f| f.flatten) {
        let read = reads.remove(i);
        reads.push(read);
    }
    let implementation = implementation(schema, &writes, &reads, &values, unknown);
    quote! {
        #[allow(dead_code)]
        #vis struct #attrs #generics (::core::marker::PhantomData<fn() -> #ident #tg>) #wc;
        #[allow(dead_code)]
        impl #ig #attrs #tg #wc { #(#handles)* }
        #implementation
    }
}

pub fn unknown_check(schema: &Record) -> TokenStream {
    if schema.allow_unknown {
        return quote!();
    }
    let path = &schema.crate_path;
    let names = schema
        .fields
        .iter()
        .filter(|f| !f.flatten)
        .map(|field| &field.name);
    let variants = crate::variants_flatten::known(schema);
    let exclude = schema
        .fields
        .iter()
        .any(|f| f.flatten)
        .then(|| quote!(&& !(#variants)));
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    quote! {
        if let ::core::option::Option::Some(name) = #item.keys().filter(|name| ![#(#names,)*].contains(&name.as_str()) #exclude).min() {
            return ::core::result::Result::Err(#cx.at(#path::PathSegment::Attribute(name.clone())).error(#path::DecodeFault::UnknownAttribute));
        }
    }
}

pub fn implementation(
    schema: &Record,
    writes: &[TokenStream],
    reads: &[TokenStream],
    values: &[TokenStream],
    unknown: TokenStream,
) -> TokenStream {
    let (ident, path) = (&schema.ident, &schema.crate_path);
    let generics = bounds(schema);
    let (ig, tg, wc) = generics.split_for_impl();
    let item = format_ident!("item", span = Span::mixed_site());
    let cx = format_ident!("cx", span = Span::mixed_site());
    let validation = crate::variants_flatten::validation(schema);
    let hook = schema.validate.as_ref().map(|hook| {
        quote_spanned!(hook.span()=>
            let __tabulary_validate: fn(&Self) -> ::core::result::Result<(), #path::ValidationFault<Self>> = #hook;
        )
    });
    let encode_hook = schema.validate.as_ref().map(|_| {
        quote! {
            #hook
            __tabulary_validate(self).map_err(|fault| fault.into_encode_error(&#cx))?;
        }
    });
    // Preserve the existing expansion exactly when there is no hook.
    let construction = if schema.validate.is_some() {
        quote! {
            let __tabulary_record = Self { #(#values,)* };
            #hook
            __tabulary_validate(&__tabulary_record).map_err(|fault| fault.into_decode_error(&#cx))?;
            #unknown
            ::core::result::Result::Ok(__tabulary_record)
        }
    } else {
        quote! { #unknown ::core::result::Result::Ok(Self { #(#values,)* }) }
    };
    let projection_fields = crate::projection_source::fields(schema);
    let declaration = (schema.generics.params.is_empty()
        && schema.fields.iter().any(|f| f.flatten))
    .then(|| quote!(const _: () = { #validation };));
    let descriptors = schema.fields.iter().enumerate().map(|(i, field)| {
        if field.flatten { return crate::variants_flatten::descriptor(schema, i); }
        let (ty, name) = (&field.ty, &field.name);
        let codec = field.codec(path);
        let presence = if field.optional { quote!(#path::Optional) } else { quote!(#path::Required) };
        let access = crate::key_expand::access(schema, i);
        quote!(#path::__private::field::<Self, #ty, #presence, #access, #codec>(#name, #i).descriptor())
    });
    quote! {
        #projection_fields
        #declaration
        impl #ig #path::Document for #ident #tg #wc {
            const NAME: &'static str = ::core::stringify!(#ident);
            const FIELDS: &'static [#path::FieldDescriptor] = &[#(#descriptors,)*];
            fn encode_map(&self, #cx: &mut #path::EncodeContext) -> ::core::result::Result<#path::Item, #path::EncodeError> {
                let #cx = #cx.in_document(<Self as #path::Document>::NAME);
                #validation
                #encode_hook
                let mut #item = #path::Item::new();
                #(#writes)*
                ::core::result::Result::Ok(#item)
            }
            fn decode_map(#item: &#path::Item, #cx: &mut #path::DecodeContext) -> ::core::result::Result<Self, #path::DecodeError> {
                let #cx = #cx.in_document(<Self as #path::Document>::NAME);
                #validation
                #(#reads)*
                #construction
            }
        }
        impl #ig #path::Value for #ident #tg #wc {
            type Wire = #path::wire::Map;
            fn encode_value(&self, cx: &mut #path::EncodeContext) -> ::core::result::Result<#path::__private::AttributeValue, #path::EncodeError> {
                let mut cx = cx.in_document(<Self as #path::Document>::NAME).enter_container()?;
                <Self as #path::Document>::encode_map(self, &mut cx).map(#path::__private::AttributeValue::M)
            }
            fn decode_value(value: &#path::__private::AttributeValue, cx: &mut #path::DecodeContext) -> ::core::result::Result<Self, #path::DecodeError> {
                let mut cx = cx.in_document(<Self as #path::Document>::NAME);
                let #path::__private::AttributeValue::M(item) = value else {
                    return ::core::result::Result::Err(#path::__private::wrong_type(value, #path::DynamoType::M, &cx));
                };
                cx = cx.enter_container()?;
                <Self as #path::Document>::decode_map(item, &mut cx)
            }
        }
    }
}
