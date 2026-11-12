use crate::{
    index_parse::{Declaration, Key, Projection},
    key_expand, parse,
    validate::{Field, Record},
};
use std::collections::HashSet;
use syn::{FieldsNamed, Ident, LitStr, ext::IdentExt};

pub struct Index {
    pub ident: Ident,
    pub name: LitStr,
    pub local: bool,
    pub sparse: bool,
    pub partition: usize,
    pub sort: Option<usize>,
    pub include: Option<Vec<String>>,
    pub all: bool,
}
pub fn indexes(
    record: &mut Record,
    declarations: Vec<Declaration>,
    fields: &FieldsNamed,
) -> syn::Result<()> {
    let (mut names, mut handles) = (HashSet::new(), HashSet::new());
    for declaration in declarations {
        let Declaration {
            ident,
            name,
            local,
            sparse,
            partition,
            sort,
            projection,
        } = declaration;
        if name.value().is_empty() || !names.insert(name.value()) {
            return Err(syn::Error::new_spanned(
                name,
                "empty or duplicate index name",
            ));
        }
        if !handles.insert(parse::rename(
            ident.to_string().trim_start_matches("r#"),
            "SCREAMING_SNAKE_CASE",
        )) {
            return Err(syn::Error::new_spanned(
                ident,
                "duplicate generated index handle",
            ));
        }
        let partition = key(record, partition, fields, &ident, true, sparse)?;
        let sort = sort
            .map(|k| key(record, k, fields, &ident, false, sparse))
            .transpose()?;
        if sparse
            && !std::iter::once(partition).chain(sort).any(|key| {
                crate::sparse_expand::inputs(record, key).any(|i| record.fields[i].optional)
            })
        {
            return Err(syn::Error::new_spanned(
                &ident,
                "sparse index needs at least one optional input",
            ));
        }
        if Some(partition) == sort {
            return Err(syn::Error::new_spanned(
                &ident,
                "index partition and sort must be distinct",
            ));
        }
        if local
            && (record.sort.is_none()
                || partition != record.partition
                || sort.is_none()
                || sort == record.sort)
        {
            return Err(syn::Error::new_spanned(
                &ident,
                "local index requires the base partition and a distinct sort on a table with a sort key",
            ));
        }
        let all = matches!(projection, Projection::All);
        let include = if let Projection::Include(entries) = projection {
            let mut names = Vec::new();
            if entries.is_empty() {
                return Err(syn::Error::new_spanned(
                    &ident,
                    "include needs at least one field",
                ));
            }
            for entry in entries {
                let field = field(record, &entry)?;
                if [Some(record.partition), record.sort, Some(partition), sort]
                    .contains(&Some(field))
                {
                    return Err(syn::Error::new_spanned(
                        entry,
                        "include accepts only non-key attributes",
                    ));
                }
                if names.contains(&record.fields[field].name) {
                    return Err(syn::Error::new_spanned(entry, "duplicate include field"));
                }
                names.push(record.fields[field].name.clone());
            }
            Some(names)
        } else {
            None
        };
        record.indexes.push(Index {
            ident,
            name,
            local,
            sparse,
            partition,
            sort,
            include,
            all,
        });
    }
    for i in 0..record.fields.len() {
        if key_expand::template(record, i).is_some() {
            record.fields[i].optional = crate::sparse_expand::conditional(record, i);
        }
    }
    Ok(())
}
fn field(record: &Record, ident: &Ident) -> syn::Result<usize> {
    record
        .fields
        .iter()
        .enumerate()
        .find(|(i, f)| {
            f.ident.unraw() == ident.unraw()
                && !f.flatten
                && key_expand::template(record, *i).is_none()
        })
        .map(|(i, _)| i)
        .ok_or_else(|| syn::Error::new_spanned(ident, "expected an ordinary Rust field"))
}
fn key(
    record: &mut Record,
    key: Key,
    fields: &FieldsNamed,
    index: &Ident,
    partition: bool,
    sparse: bool,
) -> syn::Result<usize> {
    let max = if partition { 2048 } else { 1024 };
    match key {
        Key::Field(ident) => {
            let i = field(record, &ident)?;
            required(&record.fields[i], sparse)?;
            Ok(i)
        }
        Key::Template(decl) => {
            // The primary grammar validates boundaries on the logical inner type.
            let mut inner = fields.clone();
            for field in &mut inner.named {
                if let Some(ty) = crate::validate::optional(&field.ty) {
                    field.ty = ty;
                }
            }
            let template = crate::template::parse(&decl.template, &inner)?;
            for i in key_expand::components(&template.parts) {
                required(&record.fields[i], sparse)?;
            }
            let name = decl.name.value();
            if name.is_empty() {
                return Err(syn::Error::new_spanned(
                    decl.name,
                    "empty synthesized index key name",
                ));
            }
            if let Some(i) = record.fields.iter().position(|f| f.name == name) {
                if key_expand::template(record, i).is_some_and(|t| t.parts == template.parts) {
                    record.fields[i].key_limit = Some(record.fields[i].key_limit.unwrap().min(max));
                    return Ok(i);
                }
                return Err(syn::Error::new_spanned(
                    decl.name,
                    "physical index key name collides with a field or different template",
                ));
            }
            let ident = quote::format_ident!(
                "{}_{}",
                parse::rename(&index.to_string(), "snake_case"),
                if partition { "partition" } else { "sort" }
            );
            if record.fields.iter().any(|f| {
                parse::rename(&f.ident.to_string(), "SCREAMING_SNAKE_CASE")
                    == parse::rename(&ident.to_string(), "SCREAMING_SNAKE_CASE")
            }) {
                return Err(syn::Error::new_spanned(
                    index,
                    "duplicate generated attribute handle",
                ));
            }
            let i = record.fields.len();
            record.fields.push(Field {
                ident,
                vis: record.vis.clone(),
                ty: syn::parse_quote!(::std::string::String),
                name,
                optional: false,
                key_limit: Some(max),
                default: None,
                flatten: false,
            });
            record.templates.push((i, template));
            Ok(i)
        }
    }
}
fn required(field: &Field, sparse: bool) -> syn::Result<()> {
    let message = if field.optional && !sparse {
        "non-sparse index inputs must be required"
    } else if field.default.is_some() {
        "index key inputs cannot have defaults"
    } else {
        return Ok(());
    };
    Err(syn::Error::new_spanned(&field.ident, message))
}

#[cfg(test)]
mod tests {
    #[test]
    fn invalid_declarations_never_reach_expansion() {
        for index in [
            "ident = ByValue, name = \"\", kind = global, partition = value, project = all",
            "ident = ByValue, name = \"by_value\", kind = other, partition = value, project = all",
            "ident = ByValue, name = \"by_value\", kind = global, partition = value",
            "ident = ByValue, name = \"by_value\", kind = global, partition = absent, project = all",
            "ident = ByValue, name = \"by_value\", kind = global, partition = value, sort = value, project = all",
            "ident = ByValue, name = \"by_value\", kind = global, partition = value, project = include()",
            "ident = ByValue, name = \"by_value\", kind = global, partition = value, project = include(absent)",
            "ident = ByValue, name = \"by_value\", kind = global, partition = value, project = include(note, note)",
            "ident = ByValue, name = \"by_value\", kind = global, kind = local, partition = value, project = all",
        ] {
            let input = syn::parse_str(&format!("#[tabulary(rename_all = \"snake_case\", index({index}))] struct Order {{ #[tabulary(partition_key)] id: String, value: String, note: String }}")).unwrap();
            assert!(crate::validate::record(input).is_err(), "{index}");
        }
    }
}
