use syn::{Attribute, LitStr};

#[derive(Default)]
pub struct Options {
    pub rename: Option<LitStr>,
    pub rename_all: Option<LitStr>,
    pub crate_path: Option<LitStr>,
}

pub fn options(attrs: &[Attribute], container: bool) -> syn::Result<Options> {
    let mut result = Options::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("tabulary")) {
        attr.parse_nested_meta(|meta| {
            let slot = if container && meta.path.is_ident("rename_all") {
                &mut result.rename_all
            } else if container && meta.path.is_ident("crate") {
                &mut result.crate_path
            } else if !container && meta.path.is_ident("rename") {
                &mut result.rename
            } else {
                return Err(meta.error("unsupported StringEnum option"));
            };
            if slot.is_some() {
                return Err(meta.error("duplicate StringEnum option"));
            }
            *slot = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(result)
}
