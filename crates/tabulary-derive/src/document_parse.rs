use syn::Attribute;

pub fn options(attrs: &[Attribute], container: bool) -> syn::Result<crate::parse::Options> {
    let opts = crate::parse::options(attrs, container)?;
    if opts.partition_key || opts.sort_key {
        return Err(syn::Error::new_spanned(
            &attrs[0],
            "Document fields cannot declare keys",
        ));
    }
    Ok(opts)
}
