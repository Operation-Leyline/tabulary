//! Parse and statically check a multi-object table enum.

use std::collections::HashMap;

use syn::{
    Data, DeriveInput, Fields, Ident, LitStr, Path, Type, Visibility, meta::ParseNestedMeta,
};

fn error(tokens: impl quote::ToTokens, message: &str) -> syn::Error {
    syn::Error::new_spanned(tokens, message)
}

#[derive(Clone)]
pub enum Pattern {
    Any,
    Prefix(LitStr),
    Exact(LitStr),
}

pub struct Variant {
    pub ident: Ident,
    pub ty: Type,
    pub partition: Pattern,
    pub sort: Pattern,
}

pub struct MultiObject {
    pub ident: Ident,
    pub vis: Visibility,
    pub crate_path: Path,
    pub partition: LitStr,
    pub sort: Option<LitStr>,
    pub variants: Vec<Variant>,
}

fn string_slot(meta: &ParseNestedMeta<'_>, slot: &mut Option<LitStr>) -> syn::Result<()> {
    if slot.is_some() {
        return Err(meta.error("duplicate MultiObject option"));
    }
    let value: LitStr = meta.value()?.parse()?;
    if value.value().is_empty() {
        return Err(error(&value, "empty key attribute name"));
    }
    *slot = Some(value);
    Ok(())
}

fn pattern(meta: &ParseNestedMeta<'_>, slot: &mut Option<Pattern>) -> syn::Result<()> {
    if slot.is_some() {
        return Err(meta.error("duplicate key discriminator"));
    }
    let mut found = None;
    meta.parse_nested_meta(|nested| {
        if found.is_some() {
            return Err(nested.error("a key discriminator has exactly one prefix or exact"));
        }
        let exact = nested.path.is_ident("exact");
        if !exact && !nested.path.is_ident("prefix") {
            return Err(nested.error("expected prefix = \"...\" or exact = \"...\""));
        }
        let text: LitStr = nested.value()?.parse()?;
        if text.value().is_empty() {
            return Err(error(&text, "empty key discriminator"));
        }
        found = Some(if exact {
            Pattern::Exact(text)
        } else {
            Pattern::Prefix(text)
        });
        Ok(())
    })?;
    *slot = Some(found.ok_or_else(|| meta.error("expected prefix = \"...\" or exact = \"...\""))?);
    Ok(())
}

fn starts(a: &str, b: &str) -> bool {
    a.starts_with(b)
}

/// The same rule as the runtime `KeyPattern::overlaps`.
fn overlaps(a: &Pattern, b: &Pattern) -> bool {
    match (a, b) {
        (Pattern::Any, _) | (_, Pattern::Any) => true,
        (Pattern::Prefix(a), Pattern::Prefix(b)) => {
            starts(&a.value(), &b.value()) || starts(&b.value(), &a.value())
        }
        (Pattern::Prefix(p), Pattern::Exact(e)) | (Pattern::Exact(e), Pattern::Prefix(p)) => {
            starts(&e.value(), &p.value())
        }
        (Pattern::Exact(a), Pattern::Exact(b)) => a.value() == b.value(),
    }
}

pub fn multi_object(input: DeriveInput) -> syn::Result<MultiObject> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(error(
            &input.generics,
            "a multi-object table enum cannot be generic",
        ));
    }
    let (mut partition, mut sort, mut crate_path) = (None, None, None);
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("tabulary")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("partition") {
                string_slot(&meta, &mut partition)
            } else if meta.path.is_ident("sort") {
                string_slot(&meta, &mut sort)
            } else if meta.path.is_ident("crate") {
                if crate_path.is_some() {
                    return Err(meta.error("duplicate MultiObject option"));
                }
                crate_path = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("unsupported MultiObject option"))
            }
        })?;
    }
    let partition = partition.ok_or_else(|| {
        error(
            &input.ident,
            "MultiObject needs #[tabulary(partition = \"...\")] naming the shared partition key",
        )
    })?;
    if sort
        .as_ref()
        .is_some_and(|s| s.value() == partition.value())
    {
        return Err(error(
            sort.as_ref().unwrap(),
            "sort and partition keys must differ",
        ));
    }
    let Data::Enum(data) = input.data else {
        return Err(error(&input.ident, "MultiObject requires an enum"));
    };
    if data.variants.is_empty() {
        return Err(error(
            &input.ident,
            "MultiObject requires at least one variant",
        ));
    }
    let mut variants: Vec<Variant> = Vec::new();
    let mut members = HashMap::new();
    for variant in data.variants {
        let ty = match &variant.fields {
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => fields.unnamed[0].ty.clone(),
            _ => {
                return Err(error(
                    &variant,
                    "each MultiObject variant holds exactly one record type, as `Name(Record)`",
                ));
            }
        };
        let member = quote::quote!(#ty).to_string();
        if let Some(first) = members.insert(member, variant.ident.clone()) {
            return Err(error(
                &ty,
                &format!("this record type is already the member of variant `{first}`"),
            ));
        }
        let (mut p, mut s) = (None, None);
        for attr in variant
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("tabulary"))
        {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("partition") {
                    pattern(&meta, &mut p)
                } else if meta.path.is_ident("sort") {
                    if sort.is_none() {
                        return Err(meta.error(
                            "a sort discriminator needs #[tabulary(sort = \"...\")] on the enum",
                        ));
                    }
                    pattern(&meta, &mut s)
                } else {
                    Err(meta.error("expected partition(...) or sort(...)"))
                }
            })?;
        }
        let checked = Variant {
            ident: variant.ident,
            ty,
            partition: p.unwrap_or(Pattern::Any),
            sort: s.unwrap_or(Pattern::Any),
        };
        for earlier in &variants {
            if overlaps(&checked.partition, &earlier.partition)
                && overlaps(&checked.sort, &earlier.sort)
            {
                return Err(error(
                    &checked.ident,
                    &format!(
                        "variant `{}` is not disjoint from `{}`: one stored key could match both; \
                         give them non-overlapping partition or sort prefixes",
                        checked.ident, earlier.ident
                    ),
                ));
            }
        }
        variants.push(checked);
    }
    Ok(MultiObject {
        ident: input.ident,
        vis: input.vis,
        crate_path: crate::document_validate::crate_path(crate_path)?,
        partition,
        sort,
        variants,
    })
}
