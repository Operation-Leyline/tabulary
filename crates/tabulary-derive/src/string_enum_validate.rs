use std::collections::HashSet;

use syn::{Data, DeriveInput, Fields, Generics, Ident, Path};

use crate::{parse::rename, string_enum_parse::options};

pub struct StringEnum {
    pub ident: Ident,
    pub generics: Generics,
    pub crate_path: Path,
    pub variants: Vec<(Ident, String)>,
}

pub fn string_enum(input: DeriveInput) -> syn::Result<StringEnum> {
    let opts = options(&input.attrs, true)?;
    let rule = opts.rename_all.as_ref().map(|rule| rule.value());
    if let Some(rule) = &opts.rename_all
        && !matches!(
            rule.value().as_str(),
            "snake_case" | "camelCase" | "PascalCase" | "SCREAMING_SNAKE_CASE"
        )
    {
        return Err(syn::Error::new_spanned(rule, "unsupported rename_all rule"));
    }
    let Data::Enum(data) = input.data else {
        return Err(syn::Error::new_spanned(
            input.ident,
            "StringEnum requires an enum",
        ));
    };
    let mut names = HashSet::new();
    let mut variants = Vec::new();
    let mut errors: Option<syn::Error> = None;
    for variant in data.variants {
        let checked = (|| {
            if !matches!(variant.fields, Fields::Unit) {
                return Err(syn::Error::new_spanned(
                    &variant,
                    "StringEnum requires unit variants",
                ));
            }
            let opts = options(&variant.attrs, false)?;
            let name = match &opts.rename {
                Some(name) => name.value(),
                None => rename(
                    variant.ident.to_string().trim_start_matches("r#"),
                    rule.as_deref().ok_or_else(|| {
                        syn::Error::new_spanned(
                            &variant.ident,
                            "variant needs rename or container rename_all",
                        )
                    })?,
                ),
            };
            let span = opts
                .rename
                .as_ref()
                .map_or(variant.ident.span(), |name| name.span());
            if name.is_empty() {
                return Err(syn::Error::new(span, "empty StringEnum wire value"));
            }
            if !names.insert(name.clone()) {
                return Err(syn::Error::new(span, "duplicate StringEnum wire value"));
            }
            Ok((variant.ident, name))
        })();
        match checked {
            Ok(variant) => variants.push(variant),
            Err(error) => match &mut errors {
                Some(errors) => errors.combine(error),
                None => errors = Some(error),
            },
        }
    }
    if let Some(errors) = errors {
        return Err(errors);
    }
    Ok(StringEnum {
        ident: input.ident,
        generics: input.generics,
        crate_path: match opts.crate_path {
            Some(path) => path.parse()?,
            None => syn::parse_quote!(::tabulary),
        },
        variants,
    })
}
