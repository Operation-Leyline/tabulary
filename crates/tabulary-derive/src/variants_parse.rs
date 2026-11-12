use syn::{Attribute, LitStr, Path};

#[derive(Default)]
pub struct Options {
    pub tag: Option<LitStr>,
    pub rename: Option<LitStr>,
    pub rename_all: Option<LitStr>,
    pub unknown: Option<LitStr>,
    pub crate_path: Option<LitStr>,
    pub generated_prefix: Option<LitStr>,
    pub with: Option<Path>,
    pub default: Option<Path>,
}

pub fn options(attrs: &[Attribute], level: u8) -> syn::Result<Options> {
    let mut opts = Options::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("tabulary")) {
        attr.parse_nested_meta(|meta| {
            if level == 2 && (meta.path.is_ident("with") || meta.path.is_ident("default")) {
                let slot = if meta.path.is_ident("with") {
                    &mut opts.with
                } else {
                    &mut opts.default
                };
                if slot.is_some() {
                    return Err(meta.error("duplicate payload codec or default"));
                }
                *slot = Some(meta.value()?.parse()?);
                return Ok(());
            }
            let slot = match (
                level,
                meta.path.get_ident().map(ToString::to_string).as_deref(),
            ) {
                (0, Some("tag")) => &mut opts.tag,
                (0, Some("rename_all")) => &mut opts.rename_all,
                (0, Some("unknown")) => &mut opts.unknown,
                (0, Some("crate")) => &mut opts.crate_path,
                (0, Some("generated_prefix")) => &mut opts.generated_prefix,
                (1 | 2, Some("rename")) => &mut opts.rename,
                _ => return Err(meta.error("unsupported Variants option")),
            };
            if slot.is_some() {
                return Err(meta.error("duplicate Variants option"));
            }
            *slot = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(opts)
}

pub fn name(id: &syn::Ident, explicit: Option<LitStr>, rule: Option<&str>) -> syn::Result<String> {
    let name = match explicit {
        Some(name) => name.value(),
        None => crate::parse::rename(
            id.to_string().trim_start_matches("r#"),
            rule.ok_or_else(|| {
                syn::Error::new_spanned(id, "variant or field needs rename or container rename_all")
            })?,
        ),
    };
    if name.is_empty() {
        return Err(syn::Error::new_spanned(id, "empty wire name"));
    }
    Ok(name)
}
