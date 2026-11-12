use syn::{DeriveInput, GenericArgument, Generics, Path, PathArguments, Type};

pub fn document(input: DeriveInput) -> syn::Result<crate::validate::Record> {
    crate::validate::structure(input, false)
}

/// Bound concrete field types, preserving associated types and caller predicates.
pub fn bounds(schema: &crate::validate::Record) -> Generics {
    let mut generics = schema.generics.clone();
    let path = &schema.crate_path;
    if generics.params.is_empty() {
        return generics;
    }
    for field in &schema.fields {
        let ty = &field.ty;
        generics
            .make_where_clause()
            .predicates
            .push(syn::parse_quote_spanned!(field.ident.span()=> #ty: #path::Value));
        if field.key_limit.is_some() {
            generics
                .make_where_clause()
                .predicates
                .push(syn::parse_quote_spanned!(field.ident.span()=> #ty: ::core::clone::Clone));
        }
    }
    generics
}

pub fn owned(ty: &Type) -> syn::Result<()> {
    match ty {
        Type::Reference(_) => {
            return Err(syn::Error::new_spanned(
                ty,
                "document and record fields must be owned",
            ));
        }
        Type::Array(array) => owned(&array.elem)?,
        Type::Tuple(tuple) => {
            for ty in &tuple.elems {
                owned(ty)?;
            }
        }
        Type::Paren(paren) => owned(&paren.elem)?,
        Type::Group(group) => owned(&group.elem)?,
        Type::Path(path) => {
            if let Some(qself) = &path.qself {
                owned(&qself.ty)?;
            }
            for segment in &path.path.segments {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let GenericArgument::Type(ty) = arg {
                            owned(ty)?;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn crate_path(path: Option<syn::LitStr>) -> syn::Result<Path> {
    path.map_or_else(|| Ok(syn::parse_quote!(::tabulary)), |path| path.parse())
}
