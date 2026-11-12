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
}

pub struct Record {
    pub ident: Ident,
    pub vis: Visibility,
    pub prefix: Ident,
    pub fields: Vec<Field>,
    pub partition: usize,
    pub sort: Option<usize>,
    pub allow_unknown: bool,
}

pub fn record(input: DeriveInput) -> syn::Result<Record> {
    if !input.generics.params.is_empty() {
        return Err(error(
            input.generics,
            "generic records require milestone 12",
        ));
    }
    let opts = parse::options(&input.attrs, true)?;
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
        return Err(error(input.ident, "Record requires a struct"));
    };
    let Fields::Named(fields) = data.fields else {
        return Err(error(data.fields, "Record requires named fields"));
    };
    let mut result = Record {
        ident: input.ident,
        vis: input.vis,
        prefix,
        fields: Vec::new(),
        partition: 0,
        sort: None,
        allow_unknown,
    };
    let mut partition = None;
    let mut names = HashMap::new();
    let mut handles = HashSet::new();
    for (i, field) in fields.named.into_iter().enumerate() {
        let opts = parse::options(&field.attrs, false)?;
        let ident = field
            .ident
            .ok_or_else(|| error(&field.ty, "expected a named field"))?;
        let rust_name = ident.to_string().trim_start_matches("r#").to_owned();
        let name_span = opts
            .rename
            .as_ref()
            .map_or(ident.span(), |name| name.span());
        let name = match opts.rename {
            Some(name) => name.value(),
            None => parse::rename(
                &rust_name,
                rule.as_deref()
                    .ok_or_else(|| error(&ident, "field needs rename or container rename_all"))?,
            ),
        };
        if name.is_empty() {
            return Err(syn::Error::new(name_span, "empty wire name"));
        }
        if let Some(first_span) = names.insert(name.clone(), name_span) {
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
        if matches!(&field.ty, Type::Reference(_)) {
            return Err(error(&field.ty, "record fields must be owned"));
        }
        result.fields.push(Field {
            ident,
            vis: field.vis,
            optional: inner.is_some(),
            ty: inner.unwrap_or(field.ty),
            name,
            key_limit,
            default: opts.default,
        });
    }
    result.partition =
        partition.ok_or_else(|| error(&result.ident, "exactly one partition key is required"))?;
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
