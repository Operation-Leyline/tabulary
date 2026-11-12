use crate::variants_parse::{name as wire_name, options};
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, HashSet};
use syn::{Data, DeriveInput, Fields, Ident, Path, Type};

pub struct Payload {
    pub id: Ident,
    pub name: String,
    pub ty: Type,
    pub optional: bool,
    pub codec: proc_macro2::TokenStream,
    pub default: Option<Path>,
}
pub struct Variant {
    pub id: Ident,
    pub name: String,
    pub named: bool,
    pub fields: Vec<Payload>,
}
pub struct Variants {
    pub input: DeriveInput,
    pub path: Path,
    pub prefix: Ident,
    pub tag: String,
    pub allow_unknown: bool,
    pub variants: Vec<Variant>,
    pub names: Vec<String>,
}

pub fn variants(input: DeriveInput) -> syn::Result<Variants> {
    let opts = options(&input.attrs, 0)?;
    if input.generics.lifetimes().next().is_some() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Variants must be owned",
        ));
    }
    let tag = opts
        .tag
        .ok_or_else(|| syn::Error::new_spanned(&input.ident, "Variants requires tag"))?
        .value();
    if tag.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "empty discriminator name",
        ));
    }
    let rule = opts.rename_all.map(|r| r.value());
    if let Some(rule) = &rule
        && !matches!(
            rule.as_str(),
            "snake_case" | "camelCase" | "PascalCase" | "SCREAMING_SNAKE_CASE"
        )
    {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "unsupported rename_all rule",
        ));
    }
    let allow_unknown = match opts.unknown.map(|s| s.value()).as_deref() {
        None | Some("deny") => false,
        Some("allow") => true,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "unknown must be allow or deny",
            ));
        }
    };
    let path = crate::document_validate::crate_path(opts.crate_path)?;
    let prefix = match opts.generated_prefix {
        Some(p) => p.parse()?,
        None => input.ident.clone(),
    };
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Variants requires an enum",
        ));
    };
    let mut tags = HashSet::new();
    let mut handles = HashSet::new();
    let mut shared = BTreeMap::new();
    let mut variants = Vec::new();
    for v in &data.variants {
        let opts = options(&v.attrs, 1)?;
        let name = wire_name(&v.ident, opts.rename, rule.as_deref())?;
        if !tags.insert(name.clone())
            || !handles.insert(crate::parse::rename(
                &v.ident.to_string(),
                "SCREAMING_SNAKE_CASE",
            ))
        {
            return Err(syn::Error::new_spanned(
                &v.ident,
                "duplicate variant tag or generated tag handle",
            ));
        }
        if matches!(v.fields, Fields::Unnamed(_)) {
            return Err(syn::Error::new_spanned(
                &v.fields,
                "Variants requires unit or named-field variants",
            ));
        }
        let mut fields = Vec::new();
        let mut names = HashSet::new();
        for f in &v.fields {
            crate::document_validate::owned(&f.ty)?;
            let id = f.ident.clone().expect("named fields");
            let opts = options(&f.attrs, 2)?;
            let name = wire_name(&id, opts.rename, rule.as_deref())?;
            if name == tag || !names.insert(name.clone()) {
                return Err(syn::Error::new_spanned(
                    &id,
                    "payload overlaps discriminator or duplicates a wire name",
                ));
            }
            let inner = crate::validate::optional(&f.ty);
            let ty = inner.clone().unwrap_or_else(|| f.ty.clone());
            let codec = match opts.with {
                Some(p) => quote!(#p::Codec),
                None => quote!(#path::Native<#ty>),
            };
            let signature = (
                f.ty.to_token_stream().to_string(),
                codec.to_string(),
                opts.default.to_token_stream().to_string(),
            );
            if let Some(first) = shared.insert(name.clone(), signature.clone())
                && first != signature
            {
                return Err(syn::Error::new_spanned(
                    &id,
                    "shared payload fields require identical types, converters and presence/default rules",
                ));
            }
            fields.push(Payload {
                id,
                name,
                ty,
                optional: inner.is_some(),
                codec,
                default: opts.default,
            });
        }
        variants.push(Variant {
            id: v.ident.clone(),
            name,
            named: matches!(v.fields, Fields::Named(_)),
            fields,
        });
    }
    let names = shared.into_keys().collect();
    Ok(Variants {
        input,
        path,
        prefix,
        tag,
        allow_unknown,
        variants,
        names,
    })
}

/// Recursive payload codecs are supplied by this same implementation.
pub fn references_self(tokens: proc_macro2::TokenStream, name: &Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(id) => id == *name || id == "Self",
        proc_macro2::TokenTree::Group(group) => references_self(group.stream(), name),
        _ => false,
    })
}
