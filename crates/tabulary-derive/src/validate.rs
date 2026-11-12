use std::collections::{HashMap, HashSet};

use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, Path, PathArguments, Type, Visibility,
};

use crate::parse;

fn error(tokens: impl quote::ToTokens, message: &str) -> syn::Error {
    syn::Error::new_spanned(tokens, message)
}

pub struct Field {
    pub ident: Ident,
    pub vis: Visibility,
    pub ty: Type,
    pub name: String,
    pub optional: bool,
    pub key_limit: Option<usize>,
    pub default: Option<Path>,
    pub flatten: bool,
}

pub struct Record {
    pub ident: Ident,
    pub generics: syn::Generics,
    pub crate_path: Path,
    pub vis: Visibility,
    pub prefix: Ident,
    pub fields: Vec<Field>,
    pub partition: usize,
    pub sort: Option<usize>,
    pub allow_unknown: bool,
    pub templates: Vec<(usize, crate::template::Template)>,
    pub indexes: Vec<crate::index_validate::Index>,
    pub version: Option<usize>,
}

pub fn record(input: DeriveInput) -> syn::Result<Record> {
    structure(input, true)
}

pub fn structure(input: DeriveInput, record: bool) -> syn::Result<Record> {
    if input.generics.lifetimes().next().is_some() {
        return Err(error(
            input.generics,
            "document and record types cannot have lifetime parameters",
        ));
    }
    let options = if record {
        parse::options
    } else {
        crate::document_parse::options
    };
    let opts = options(&input.attrs, true)?;
    let allow_unknown = match opts
        .unknown
        .as_ref()
        .map(|policy| policy.value())
        .as_deref()
    {
        None | Some("deny") => false,
        Some("allow") => true,
        _ => return Err(error(opts.unknown, "unknown must be \"allow\" or \"deny\"")),
    };
    if let Some(rule) = &opts.rename_all
        && !matches!(
            rule.value().as_str(),
            "snake_case" | "camelCase" | "PascalCase" | "SCREAMING_SNAKE_CASE"
        )
    {
        return Err(error(rule, "unsupported rename_all rule"));
    }
    let rule = opts.rename_all.as_ref().map(|rule| rule.value());
    let prefix = match opts.generated_prefix {
        Some(prefix) => syn::parse_str::<Ident>(&prefix.value())
            .map_err(|_| error(prefix, "expected a Rust identifier"))?,
        None => input.ident.clone(),
    };
    let Data::Struct(data) = input.data else {
        return Err(error(input.ident, "Record and Document require a struct"));
    };
    let Fields::Named(fields) = data.fields else {
        return Err(error(
            data.fields,
            "Record and Document require named fields",
        ));
    };
    let templates = crate::template::declarations(
        opts.partition.as_ref(),
        opts.sort.as_ref(),
        &fields,
        rule.as_deref(),
    )?;
    let mut result = Record {
        ident: input.ident,
        generics: input.generics,
        crate_path: crate::document_validate::crate_path(opts.crate_path)?,
        vis: input.vis,
        prefix,
        fields: Vec::new(),
        partition: 0,
        sort: None,
        allow_unknown,
        templates: Vec::new(),
        indexes: Vec::new(),
        version: None,
    };
    let mut partition = None;
    let mut names = HashMap::new();
    let mut handles = HashSet::new();
    let mut flattened = false;
    for (i, field) in fields.named.iter().cloned().enumerate() {
        let opts = options(&field.attrs, false)?;
        let ident = field
            .ident
            .ok_or_else(|| error(&field.ty, "expected a named field"))?;
        let rust_name = ident.to_string().trim_start_matches("r#").to_owned();
        if opts.version {
            if result.version.replace(i).is_some() {
                return Err(error(&ident, "at most one version field is allowed"));
            }
            // Validate the written primitive type; aliases cannot declare a version.
            let native = match &field.ty {
                Type::Path(p) if p.qself.is_none() => {
                    let name = quote::quote!(#p).to_string().replace(' ', "");
                    matches!(
                        name.as_str(),
                        "u64"
                            | "core::primitive::u64"
                            | "::core::primitive::u64"
                            | "std::primitive::u64"
                            | "::std::primitive::u64"
                    )
                }
                _ => false,
            };
            if !native
                || opts.default.is_some()
                || opts.partition_key
                || opts.sort_key
                || opts.flatten
            {
                return Err(error(
                    &ident,
                    "version must be a required native u64 without a default, key or flatten",
                ));
            }
        }
        if opts.flatten {
            if flattened {
                return Err(error(
                    &ident,
                    "at most one flattened variants field is allowed",
                ));
            }
            flattened = true;
            if opts.partition_key
                || opts.sort_key
                || opts.default.is_some()
                || opts.rename.is_some()
                || optional(&field.ty).is_some()
            {
                return Err(error(
                    &ident,
                    "flatten cannot be optional, renamed, defaulted or a key",
                ));
            }
        }
        let name_span = opts
            .rename
            .as_ref()
            .map_or(ident.span(), |name| name.span());
        let name = match opts.rename {
            Some(name) => name.value(),
            None if opts.flatten => rust_name.clone(),
            None => parse::rename(
                &rust_name,
                rule.as_deref()
                    .ok_or_else(|| error(&ident, "field needs rename or container rename_all"))?,
            ),
        };
        if name.is_empty() {
            return Err(syn::Error::new(name_span, "empty wire name"));
        }
        if !opts.flatten
            && let Some(first_span) = names.insert(name.clone(), name_span)
        {
            let mut duplicate = syn::Error::new(name_span, format!("duplicate wire name `{name}`"));
            duplicate.combine(syn::Error::new(first_span, "first declared here"));
            return Err(duplicate);
        }
        if !handles.insert(parse::rename(&rust_name, "SCREAMING_SNAKE_CASE")) {
            return Err(error(&ident, "duplicate generated attribute handle"));
        }
        let inner = optional(&field.ty);
        let key_limit = if opts.partition_key {
            if partition.replace(i).is_some() || opts.sort_key {
                return Err(error(&ident, "duplicate or overlapping partition key"));
            }
            Some(2048)
        } else if opts.sort_key {
            if result.sort.replace(i).is_some() {
                return Err(error(&ident, "at most one sort key is allowed"));
            }
            Some(1024)
        } else {
            None
        };
        if key_limit.is_some() && inner.is_some() {
            return Err(error(&ident, "primary keys cannot be optional"));
        }
        if key_limit.is_some() && opts.default.is_some() {
            return Err(error(opts.default, "primary keys cannot have defaults"));
        }
        crate::document_validate::owned(&field.ty)?;
        result.fields.push(Field {
            ident,
            vis: field.vis,
            optional: inner.is_some(),
            ty: inner.unwrap_or(field.ty),
            name,
            key_limit,
            default: opts.default,
            flatten: opts.flatten,
        });
    }
    for ((declaration, is_partition), template) in [(opts.partition, true), (opts.sort, false)]
        .into_iter()
        .filter_map(|(decl, kind)| decl.map(|decl| (decl, kind)))
        .zip(templates)
    {
        let ident = Ident::new(
            if is_partition { "partition" } else { "sort" },
            declaration.name.span(),
        );
        if !handles.insert(ident.to_string().to_uppercase()) {
            return Err(error(&ident, "duplicate generated attribute handle"));
        }
        let index = result.fields.len();
        if is_partition {
            partition = Some(index);
        } else {
            result.sort = Some(index);
        }
        result.templates.push((index, template));
        result.fields.push(Field {
            ident,
            vis: result.vis.clone(),
            ty: syn::parse_quote!(::std::string::String),
            name: declaration.name.value(),
            optional: false,
            key_limit: Some(if is_partition { 2048 } else { 1024 }),
            default: None,
            flatten: false,
        });
    }
    if record {
        result.partition = partition
            .ok_or_else(|| error(&result.ident, "exactly one partition key is required"))?;
        crate::index_validate::indexes(&mut result, opts.indexes, &fields)?;
        if let Some(i) = result.version
            && (crate::key_expand::source(&result, i)
                || crate::key_expand::index_source(&result, i))
        {
            return Err(error(
                &result.fields[i].ident,
                "version cannot be a composite key source",
            ));
        }
    }
    Ok(result)
}

pub fn optional(ty: &Type) -> Option<Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    }
}
