use syn::{Attribute, DeriveInput, Fields, Ident, Path, Type};

pub struct Projection {
    pub input: DeriveInput,
    pub source: Type,
    pub path: Path,
    pub fields: Vec<Field>,
}
pub struct Field {
    pub id: Ident,
    pub ty: Type,
    pub handle: Path,
    pub nested: bool,
}

pub fn projection(input: DeriveInput) -> syn::Result<Projection> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Projection requires an owned, concrete view type",
        ));
    }
    let mut source = None;
    let mut path = None;
    for attr in attrs(&input.attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("record") && source.is_none() {
                source = Some(meta.value()?.parse::<Type>()?);
            } else if meta.path.is_ident("crate") && path.is_none() {
                path = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("unsupported or duplicate Projection option"));
            }
            Ok(())
        })?;
    }
    let source = source
        .ok_or_else(|| syn::Error::new_spanned(&input.ident, "Projection needs record = Source"))?;
    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Projection requires a named-field struct",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Projection requires a named-field struct",
        ));
    };
    if named.named.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Projection must select at least one field",
        ));
    }
    let mut fields = Vec::new();
    for field in &named.named {
        crate::document_validate::owned(&field.ty)?;
        let mut handle = None;
        let mut nested = false;
        for attr in attrs(&field.attrs) {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("attribute") && handle.is_none() { handle = Some(meta.value()?.parse::<Path>()?); }
                else if meta.path.is_ident("nested") && !nested { nested = true; }
                else { return Err(meta.error("Projection fields accept attribute and nested only; codecs and defaults are inherited")); }
                Ok(())
            })?;
        }
        fields.push(Field {
            id: field.ident.clone().expect("named field"),
            ty: field.ty.clone(),
            nested,
            handle: handle.ok_or_else(|| {
                syn::Error::new_spanned(
                    field,
                    "Projection field needs attribute = SourceAttrs::FIELD",
                )
            })?,
        });
    }
    Ok(Projection {
        input,
        source,
        path: crate::document_validate::crate_path(path)?,
        fields,
    })
}
fn attrs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|a| a.path().is_ident("tabulary"))
}
