use syn::{Ident, LitStr, Token, ext::IdentExt, meta::ParseNestedMeta, punctuated::Punctuated};

pub enum Key {
    Field(Ident),
    Template(crate::template::Declaration),
}
pub enum Projection {
    All,
    KeysOnly,
    Include(Vec<Ident>),
}
pub struct Declaration {
    pub ident: Ident,
    pub name: LitStr,
    pub local: bool,
    pub partition: Key,
    pub sort: Option<Key>,
    pub projection: Projection,
}

pub fn declaration(meta: &ParseNestedMeta<'_>) -> syn::Result<Declaration> {
    let (mut ident, mut name, mut kind, mut partition, mut sort, mut project) =
        (None, None, None, None, None, None);
    meta.parse_nested_meta(|m| {
        if m.path.is_ident("sparse") {
            return Err(m.error("sparse indexes arrive with milestone 17"));
        }
        if m.path.is_ident("ident") {
            put(&mut ident, m.value()?.parse()?, &m)
        } else if m.path.is_ident("name") {
            put(&mut name, m.value()?.parse()?, &m)
        } else if m.path.is_ident("kind") {
            let value: Ident = m.value()?.parse()?;
            let local = match value.to_string().as_str() {
                "global" => false,
                "local" => true,
                _ => return Err(m.error("index kind must be global or local")),
            };
            put(&mut kind, local, &m)
        } else if m.path.is_ident("partition") || m.path.is_ident("sort") {
            let key = if m.input.peek(Token![=]) {
                Key::Field(m.value()?.call(Ident::parse_any)?)
            } else {
                Key::Template(crate::template::declaration(&m)?)
            };
            let slot = if m.path.is_ident("partition") {
                &mut partition
            } else {
                &mut sort
            };
            put(slot, key, &m)
        } else if m.path.is_ident("project") {
            let input = m.value()?;
            let value: Ident = input.parse()?;
            let projection = match value.to_string().as_str() {
                "all" => Projection::All,
                "keys_only" => Projection::KeysOnly,
                "include" => {
                    let content;
                    syn::parenthesized!(content in input);
                    Projection::Include(
                        Punctuated::<Ident, Token![,]>::parse_terminated_with(
                            &content,
                            Ident::parse_any,
                        )?
                        .into_iter()
                        .collect(),
                    )
                }
                _ => return Err(m.error("expected all, keys_only or include(fields)")),
            };
            put(&mut project, projection, &m)
        } else {
            Err(m.error("unsupported index option"))
        }
    })?;
    Ok(Declaration {
        ident: ident.ok_or_else(|| meta.error("index needs ident"))?,
        name: name.ok_or_else(|| meta.error("index needs name"))?,
        local: kind.ok_or_else(|| meta.error("index needs kind"))?,
        partition: partition.ok_or_else(|| meta.error("index needs partition"))?,
        sort,
        projection: project.ok_or_else(|| meta.error("index needs project"))?,
    })
}
fn put<T>(slot: &mut Option<T>, value: T, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
    if slot.replace(value).is_some() {
        Err(meta.error("duplicate index option"))
    } else {
        Ok(())
    }
}
