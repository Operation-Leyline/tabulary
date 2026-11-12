use syn::{Attribute, LitStr, Path};

#[derive(Default)]
pub struct Options {
    pub rename: Option<LitStr>,
    pub rename_all: Option<LitStr>,
    pub generated_prefix: Option<LitStr>,
    pub unknown: Option<LitStr>,
    pub crate_path: Option<LitStr>,
    pub default: Option<Path>,
    pub with: Option<Path>,
    pub partition_key: bool,
    pub sort_key: bool,
    pub flatten: bool,
    pub ttl: bool,
    pub version: bool,
    pub partition: Option<crate::template::Declaration>,
    pub sort: Option<crate::template::Declaration>,
    pub indexes: Vec<crate::index_parse::Declaration>,
}

pub fn options(attrs: &[Attribute], container: bool) -> syn::Result<Options> {
    let mut result = Options::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("tabulary")) {
        attr.parse_nested_meta(|meta| {
            let option = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("expected an option name"))?
                .to_string();
            let slot = match (container, option.as_str()) {
                (true, "index") => {
                    result.indexes.push(crate::index_parse::declaration(&meta)?);
                    return Ok(());
                }
                (true, "rename_all") => &mut result.rename_all,
                (true, "generated_prefix") => &mut result.generated_prefix,
                (true, "unknown") => &mut result.unknown,
                (true, "crate") => &mut result.crate_path,
                (true, "partition" | "sort") => {
                    let slot = if option == "partition" {
                        &mut result.partition
                    } else {
                        &mut result.sort
                    };
                    if slot.is_some() {
                        return Err(meta.error("duplicate composite key declaration"));
                    }
                    *slot = Some(crate::template::declaration(&meta)?);
                    return Ok(());
                }
                (false, "rename") => &mut result.rename,
                (false, "ttl") => {
                    if result.ttl {
                        return Err(meta.error("duplicate ttl option"));
                    }
                    result.ttl = true;
                    return Ok(());
                }
                (false, "version") => {
                    if result.version {
                        return Err(meta.error("duplicate version option"));
                    }
                    result.version = true;
                    return Ok(());
                }
                (false, "flatten") => {
                    if result.flatten {
                        return Err(meta.error("duplicate flatten option"));
                    }
                    result.flatten = true;
                    return Ok(());
                }
                (false, "default" | "with") => {
                    let slot = if option == "with" {
                        &mut result.with
                    } else {
                        &mut result.default
                    };
                    if slot.is_some() {
                        return Err(meta.error(format!("duplicate {option} option")));
                    }
                    *slot = Some(meta.value()?.parse()?);
                    return Ok(());
                }
                (false, "partition_key" | "sort_key") => {
                    let flag = if option == "partition_key" {
                        &mut result.partition_key
                    } else {
                        &mut result.sort_key
                    };
                    if *flag {
                        return Err(meta.error("duplicate key annotation"));
                    }
                    *flag = true;
                    return Ok(());
                }
                _ => return Err(meta.error("unsupported tabulary option")),
            };
            if slot.is_some() {
                return Err(meta.error(format!("duplicate {option} option")));
            }
            *slot = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(result)
}

pub fn rename(name: &str, rule: &str) -> String {
    let chars: Vec<_> = name.chars().collect();
    let mut words = String::new();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase()
            && i > 0
            && chars[i - 1] != '_'
            && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|c| c.is_lowercase()))
        {
            words.push('_');
        }
        words.extend(c.to_lowercase());
    }
    match rule {
        "snake_case" => words,
        "SCREAMING_SNAKE_CASE" => words.to_uppercase(),
        _ => words
            .split('_')
            .filter(|w| !w.is_empty())
            .enumerate()
            .map(|(i, w)| {
                if rule == "camelCase" && i == 0 {
                    return w.to_owned();
                }
                let mut chars = w.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().to_string() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_cover_all_rules_and_acronyms() {
        for (rule, expected) in [
            ("snake_case", "http_order_ref"),
            ("camelCase", "httpOrderRef"),
            ("PascalCase", "HttpOrderRef"),
            ("SCREAMING_SNAKE_CASE", "HTTP_ORDER_REF"),
        ] {
            assert_eq!(super::rename("HTTP_order_ref", rule), expected);
        }
    }
}
