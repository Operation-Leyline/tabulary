//! Composite key grammar and checked template representation.

use std::collections::HashSet;

use syn::{FieldsNamed, LitStr, ext::IdentExt, meta::ParseNestedMeta, parse::Parser};

use crate::{parse as attributes, validate::optional};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Hex,
    Dec,
    Dec20,
    Text,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Part {
    Literal(String),
    Component {
        field: usize,
        codec: Codec,
        width: Option<usize>,
    },
}

pub struct Template {
    pub parts: Vec<Part>,
}

pub struct Declaration {
    pub name: LitStr,
    pub template: LitStr,
}

pub fn declaration(meta: &ParseNestedMeta<'_>) -> syn::Result<Declaration> {
    let (mut name, mut template) = (None, None);
    meta.parse_nested_meta(|nested| {
        let slot = if nested.path.is_ident("name") {
            &mut name
        } else if nested.path.is_ident("template") {
            &mut template
        } else {
            return Err(nested.error("expected name or template"));
        };
        if slot.is_some() {
            return Err(nested.error("duplicate composite key option"));
        }
        *slot = Some(nested.value()?.parse()?);
        Ok(())
    })?;
    Ok(Declaration {
        name: name.ok_or_else(|| meta.error("composite key needs name"))?,
        template: template.ok_or_else(|| meta.error("composite key needs template"))?,
    })
}

/// Validate before the milestone-15 expansion gate; never silently ignore a key.
pub fn declarations(
    partition: Option<&Declaration>,
    sort: Option<&Declaration>,
    fields: &FieldsNamed,
    rule: Option<&str>,
) -> syn::Result<()> {
    let mut names = HashSet::new();
    for (decl, is_partition) in [(partition, true), (sort, false)] {
        let Some(decl) = decl else { continue };
        if decl.name.value().is_empty() || !names.insert(decl.name.value()) {
            return Err(syn::Error::new_spanned(
                &decl.name,
                "empty or duplicate synthesized key name",
            ));
        }
        parse(&decl.template, fields)?;
        for field in &fields.named {
            let opts = attributes::options(&field.attrs, false)?;
            if (is_partition && opts.partition_key) || (!is_partition && opts.sort_key) {
                return Err(syn::Error::new_spanned(
                    field,
                    "field and template declare the same primary key",
                ));
            }
            let ident = field.ident.as_ref().expect("named field");
            let rust_name = ident.to_string();
            let wire_name = opts.rename.map(|name| name.value()).or_else(|| {
                rule.map(|rule| attributes::rename(rust_name.trim_start_matches("r#"), rule))
            });
            if wire_name.as_deref() == Some(decl.name.value().as_str()) {
                return Err(syn::Error::new_spanned(
                    field,
                    "field wire name collides with synthesized key",
                ));
            }
        }
    }
    if let Some(decl) = partition.or(sort) {
        return Err(syn::Error::new_spanned(
            &decl.template,
            "template validated; composite key expansion requires milestone 15",
        ));
    }
    Ok(())
}

pub fn parse(value: &LitStr, fields: &FieldsNamed) -> syn::Result<Template> {
    let fail = |message| syn::Error::new_spanned(value, message);
    let text = value.value();
    if text.is_empty() {
        return Err(fail("empty key template"));
    }
    let mut chars = text.chars().peekable();
    let mut literal = String::new();
    let mut template = Template { parts: Vec::new() };
    let mut seen = HashSet::new();
    while let Some(ch) = chars.next() {
        match ch {
            '{' | '}' if chars.peek() == Some(&ch) => {
                chars.next();
                literal.push(ch);
            }
            '}' => return Err(fail("unescaped closing brace in key template")),
            '{' => {
                if !literal.is_empty() {
                    template
                        .parts
                        .push(Part::Literal(std::mem::take(&mut literal)));
                }
                let mut component = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        None | Some('{') => {
                            return Err(fail("unclosed or nested template component"));
                        }
                        Some(ch) => component.push(ch),
                    }
                }
                let (name, codec) = component
                    .split_once(':')
                    .ok_or_else(|| fail("expected {field:codec}"))?;
                let ident = syn::Ident::parse_any
                    .parse_str(name)
                    .map_err(|_| fail("expected a Rust field identifier in template"))?;
                let rust_name = ident.to_string();
                if rust_name != name {
                    return Err(fail("expected a Rust field identifier in template"));
                }
                let name = rust_name.trim_start_matches("r#");
                let (index, field) = fields
                    .named
                    .iter()
                    .enumerate()
                    .find(|(_, field)| {
                        field
                            .ident
                            .as_ref()
                            .is_some_and(|id| id.to_string().trim_start_matches("r#") == name)
                    })
                    .ok_or_else(|| fail("unknown template field"))?;
                if !seen.insert(index) {
                    return Err(fail("duplicate template component reference"));
                }
                let codec = match codec {
                    "hex" => Codec::Hex,
                    "dec" => Codec::Dec,
                    "dec20" => Codec::Dec20,
                    "text" => Codec::Text,
                    _ => {
                        return Err(fail(
                            "unknown component codec; expected hex, dec, dec20 or text",
                        ));
                    }
                };
                if optional(&field.ty).is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "primary template sources cannot be optional",
                    ));
                }
                if attributes::options(&field.attrs, false)?.default.is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "template sources cannot have defaults",
                    ));
                }
                let width = match codec {
                    Codec::Dec20 => Some(20),
                    Codec::Hex => hex_width(&field.ty),
                    _ => None,
                };
                template.parts.push(Part::Component {
                    field: index,
                    codec,
                    width,
                });
            }
            _ => literal.push(ch),
        }
    }
    if !literal.is_empty() {
        template.parts.push(Part::Literal(literal));
    }
    // A run between literals can have one variable component: the remaining
    // component boundaries follow from their fixed byte widths.
    let mut run = Vec::new();
    let mut variable = false;
    for part in &template.parts {
        match part {
            Part::Component { codec, width, .. } => {
                if width.is_none() && variable {
                    return Err(fail(
                        "adjacent variable-width components have ambiguous boundaries",
                    ));
                }
                variable |= width.is_none();
                run.push(codec);
            }
            Part::Literal(separator) => {
                // Checking the first character also excludes overlapping delimiter
                // matches that start inside a component and end in the literal.
                let first = separator.chars().next().expect("nonempty literal");
                if variable
                    && run.iter().any(|codec| match codec {
                        Codec::Hex => first.is_ascii_digit() || ('a'..='f').contains(&first),
                        Codec::Dec => first.is_ascii_digit() || first == '-',
                        Codec::Dec20 => first.is_ascii_digit(),
                        Codec::Text => false, // formatting reserves its first character
                    })
                {
                    return Err(fail(
                        "separator is admitted by a component codec; use a delimiter outside its alphabet",
                    ));
                }
                run.clear();
                variable = false;
            }
        }
    }
    Ok(template)
}

fn hex_width(ty: &syn::Type) -> Option<usize> {
    let syn::Type::Array(array) = ty else {
        return None;
    };
    let syn::Type::Path(element) = &*array.elem else {
        return None;
    };
    if !element.path.is_ident("u8") {
        return None;
    }
    let syn::Expr::Lit(length) = &array.len else {
        return None;
    };
    let syn::Lit::Int(length) = &length.lit else {
        return None;
    };
    length.base10_parse::<usize>().ok()?.checked_mul(2)
}

#[cfg(test)]
#[path = "template_tests.rs"]
mod tests;
