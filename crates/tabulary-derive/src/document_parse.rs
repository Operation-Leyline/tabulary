use syn::Attribute;

pub fn options(attrs: &[Attribute], container: bool) -> syn::Result<crate::parse::Options> {
    let opts = crate::parse::options(attrs, container)?;
    if opts.ttl {
        return Err(syn::Error::new_spanned(
            &attrs[0],
            "Document cannot declare TTL",
        ));
    }
    if opts.version {
        return Err(syn::Error::new_spanned(
            &attrs[0],
            "Document fields cannot declare versions",
        ));
    }
    if !opts.indexes.is_empty() {
        return Err(syn::Error::new_spanned(
            &attrs[0],
            "Document cannot declare indexes",
        ));
    }
    if opts.partition_key || opts.sort_key {
        return Err(syn::Error::new_spanned(
            &attrs[0],
            "Document fields cannot declare keys",
        ));
    }
    if let Some(key) = opts.partition.as_ref().or(opts.sort.as_ref()) {
        return Err(syn::Error::new_spanned(
            &key.name,
            "Document cannot declare keys",
        ));
    }
    Ok(opts)
}
