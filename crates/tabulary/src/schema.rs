//! Static metadata shared by manual records, derives, and request builders.
use crate::DynamoType;

/// Whether a field may be absent. This is independent of its wire category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Required,
    Optional,
}

/// The API that owns mutations of an attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldAccess {
    Mutable,
    KeyOnly,
    VersionOnly,
    IndexSourceOnly,
    DerivedOnly,
}

/// A field's physical metadata, without a value or a runtime registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub name: &'static str,
    pub index: usize,
    pub presence: Presence,
    pub access: FieldAccess,
    pub wire: DynamoType,
}

/// Attribute positions in the owning schema, not DynamoDB key values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyDefinition {
    pub partition: usize,
    pub sort: Option<usize>,
}

/// Static physical metadata shared by record operations and table definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordSchema {
    pub name: &'static str,
    pub fields: &'static [FieldDescriptor],
    pub primary: KeyDefinition,
    pub indexes: &'static [crate::IndexDefinition],
    pub ttl: Option<usize>,
}

/// A record owns one static schema. Manual implementations are supported.
/// Document encoding and generated key interfaces remain separate contracts.
pub trait Record: Sized {
    const SCHEMA: RecordSchema;

    /// Encode with this record's document codec and estimate its item size.
    fn billed_size(&self) -> Result<crate::BilledSize, crate::EncodeError>
    where
        Self: crate::Document,
    {
        let item = self.encode_map(&mut crate::EncodeContext::new(Self::NAME))?;
        crate::BilledSize::of_item(&item)
    }

    fn table_definition() -> crate::TableDefinition<Self> {
        crate::TableDefinition::new(Self::SCHEMA)
    }
}

impl RecordSchema {
    #[doc(hidden)]
    pub const fn with_ttl(mut self, position: usize) -> Self {
        self.ttl = Some(position);
        self
    }
}
