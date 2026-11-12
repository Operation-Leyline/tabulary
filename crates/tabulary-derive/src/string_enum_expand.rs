use proc_macro2::TokenStream;
use quote::quote;

use crate::string_enum_validate::StringEnum;

pub fn string_enum(schema: &StringEnum) -> TokenStream {
    let StringEnum {
        ident,
        generics,
        crate_path: path,
        variants,
    } = schema;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let encoders = variants.iter().map(|(variant, name)| {
        quote! {
            Self::#variant => <::std::string::String as #path::Value>::encode_value(
                &::std::string::String::from(#name), cx),
        }
    });
    let decoders = variants.iter().map(|(variant, name)| {
        quote! {
            #name => ::core::result::Result::Ok(Self::#variant),
        }
    });
    quote! {
        impl #impl_generics #path::Value for #ident #type_generics #where_clause {
            type Wire = #path::wire::String;

            fn encode_value(&self, cx: &mut #path::EncodeContext)
                -> ::core::result::Result<#path::__private::AttributeValue, #path::EncodeError>
            {
                match *self { #(#encoders)* }
            }

            fn decode_value(value: &#path::__private::AttributeValue, cx: &mut #path::DecodeContext)
                -> ::core::result::Result<Self, #path::DecodeError>
            {
                let tag = <::std::string::String as #path::Value>::decode_value(value, cx)?;
                match tag.as_str() {
                    #(#decoders)*
                    _ => ::core::result::Result::Err(cx.error(#path::DecodeFault::UnknownVariant)),
                }
            }
        }
    }
}
