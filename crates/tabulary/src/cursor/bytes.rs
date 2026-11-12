//! V1: version byte, binding, then sorted scalar key entries. All lengths/counts
//! are big-endian u32; schema positions are u64. Tags below are wire constants.
use super::*;
use crate::{DynamoType, FieldAccess, IndexKind, IndexProjection, KeyDefinition, Presence};

const MAX: usize = 4 * 1024 * 1024;
const WIRES: &[DynamoType] = &[
    DynamoType::S,
    DynamoType::N,
    DynamoType::B,
    DynamoType::Bool,
    DynamoType::Null,
    DynamoType::L,
    DynamoType::M,
    DynamoType::Ss,
    DynamoType::Ns,
    DynamoType::Bs,
    DynamoType::Unknown,
];
const ACCESS: &[FieldAccess] = &[
    FieldAccess::Mutable,
    FieldAccess::KeyOnly,
    FieldAccess::VersionOnly,
    FieldAccess::IndexSourceOnly,
    FieldAccess::DerivedOnly,
];
const OPS: &[Operator] = &[
    Operator::Exists,
    Operator::NotExists,
    Operator::Eq,
    Operator::Ne,
    Operator::Lt,
    Operator::Le,
    Operator::Gt,
    Operator::Ge,
    Operator::Between,
    Operator::BeginsWith,
    Operator::Contains,
    Operator::In,
];

impl<R, I> Cursor<R, I> {
    /// Encode V1 bytes. Keys and filter constants are included without encryption.
    pub fn to_bytes(&self) -> Result<Vec<u8>, BuildError> {
        let mut w = Writer {
            bytes: Some(Vec::new()),
            len: 0,
        };
        w.byte(1)?;
        w.binding(&self.binding)?;
        let mut keys: Vec<_> = self.key.iter().collect();
        keys.sort_unstable_by_key(|(name, _)| *name);
        w.count(keys.len())?;
        for (name, value) in keys {
            w.blob(name.as_bytes())?;
            match value {
                AttributeValue::S(v) => {
                    w.byte(0)?;
                    w.blob(v.as_bytes())?;
                }
                AttributeValue::N(v) => {
                    w.byte(1)?;
                    w.blob(v.as_bytes())?;
                }
                AttributeValue::B(v) => {
                    w.byte(2)?;
                    w.blob(v.as_ref())?;
                }
                _ => return Err(BuildError::InvalidCursor),
            }
        }
        Ok(w.bytes.unwrap())
    }
}

pub(super) fn binding_size<R, I>(binding: &CursorBinding<R, I>) -> Result<(), BuildError> {
    let mut w = Writer {
        bytes: None,
        len: 5,
    };
    w.binding(binding)
}
struct Writer {
    bytes: Option<Vec<u8>>,
    len: usize,
}
impl Writer {
    fn raw(&mut self, bytes: &[u8]) -> Result<(), BuildError> {
        self.len = self
            .len
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX)
            .ok_or(BuildError::CursorTooLarge)?;
        if let Some(out) = &mut self.bytes {
            out.extend_from_slice(bytes);
        }
        Ok(())
    }
    fn byte(&mut self, b: u8) -> Result<(), BuildError> {
        self.raw(&[b])
    }
    fn count(&mut self, n: usize) -> Result<(), BuildError> {
        self.raw(
            &u32::try_from(n)
                .map_err(|_| BuildError::CursorTooLarge)?
                .to_be_bytes(),
        )
    }
    fn blob(&mut self, bytes: &[u8]) -> Result<(), BuildError> {
        self.count(bytes.len())?;
        self.raw(bytes)
    }
    fn names(&mut self, names: &[&str]) -> Result<(), BuildError> {
        self.count(names.len())?;
        for name in names {
            self.blob(name.as_bytes())?;
        }
        Ok(())
    }
    fn position(&mut self, n: usize) -> Result<(), BuildError> {
        self.raw(&(n as u64).to_be_bytes())
    }
    fn optional_position(&mut self, n: Option<usize>) -> Result<(), BuildError> {
        self.byte(u8::from(n.is_some()))?;
        if let Some(n) = n {
            self.position(n)?;
        }
        Ok(())
    }
    fn keys(&mut self, k: KeyDefinition) -> Result<(), BuildError> {
        self.position(k.partition)?;
        self.optional_position(k.sort)
    }
    fn field(&mut self, f: FieldDescriptor) -> Result<(), BuildError> {
        self.blob(f.name.as_bytes())?;
        self.position(f.index)?;
        self.byte(u8::from(f.presence == Presence::Optional))?;
        self.byte(ACCESS.iter().position(|a| *a == f.access).unwrap() as u8)?;
        self.byte(WIRES.iter().position(|w| *w == f.wire).unwrap() as u8)
    }
    fn index(&mut self, i: IndexDefinition) -> Result<(), BuildError> {
        self.blob(i.name.as_bytes())?;
        self.byte(u8::from(i.kind == IndexKind::Local))?;
        self.keys(i.keys)?;
        match i.projection {
            IndexProjection::All => self.byte(0),
            IndexProjection::KeysOnly => self.byte(1),
            IndexProjection::Include(names) => {
                self.byte(2)?;
                self.names(names)
            }
        }
    }
    fn binding<R, I>(&mut self, b: &CursorBinding<R, I>) -> Result<(), BuildError> {
        self.blob(b.table.as_bytes())?;
        self.blob(b.schema.name.as_bytes())?;
        self.count(b.schema.fields.len())?;
        for f in b.schema.fields {
            self.field(*f)?;
        }
        self.keys(b.schema.primary)?;
        self.count(b.schema.indexes.len())?;
        for i in b.schema.indexes {
            self.index(*i)?;
        }
        self.optional_position(b.schema.ttl)?;
        self.byte(u8::from(b.index.is_some()))?;
        if let Some(i) = b.index {
            self.index(i)?;
        }
        match &b.operation {
            Operation::Query(node, order) => {
                self.byte(0)?;
                self.node(node, 0)?;
                self.byte(u8::from(*order == QueryOrder::Descending))?;
            }
            Operation::Scan(segment) => {
                self.byte(1)?;
                self.byte(u8::from(segment.is_some()))?;
                if let Some((n, total)) = segment {
                    self.raw(&n.to_be_bytes())?;
                    self.raw(&total.to_be_bytes())?;
                }
            }
        }
        self.byte(u8::from(b.filter.is_some()))?;
        if let Some(node) = &b.filter {
            self.node(node, 0)?;
        }
        self.byte(u8::from(b.projection.is_some()))?;
        if let Some(names) = &b.projection {
            self.names(names)?;
        }
        self.byte(u8::from(b.consistency == ReadConsistency::Strong))
    }
    fn node(&mut self, node: &CanonicalNode, depth: usize) -> Result<(), BuildError> {
        if depth > 32 {
            return Err(BuildError::InvalidCursor);
        }
        match node {
            CanonicalNode::Predicate(op, operands) => {
                self.byte(0)?;
                self.byte(OPS.iter().position(|o| o == op).unwrap() as u8)?;
                self.count(operands.len())?;
                for operand in operands {
                    match operand {
                        CanonicalOperand::Attribute(f) => {
                            self.byte(0)?;
                            self.field(*f)?;
                        }
                        CanonicalOperand::Size(f) => {
                            self.byte(1)?;
                            self.field(*f)?;
                        }
                        CanonicalOperand::Value(v) => {
                            self.byte(2)?;
                            self.value(v, 0)?;
                        }
                    }
                }
            }
            CanonicalNode::And(a, b) | CanonicalNode::Or(a, b) => {
                self.byte(if matches!(node, CanonicalNode::And(..)) {
                    1
                } else {
                    2
                })?;
                self.node(a, depth + 1)?;
                self.node(b, depth + 1)?;
            }
            CanonicalNode::Not(a) => {
                self.byte(3)?;
                self.node(a, depth + 1)?;
            }
        }
        Ok(())
    }
    fn value(&mut self, value: &CanonicalValue, depth: usize) -> Result<(), BuildError> {
        use CanonicalValue as V;
        match value {
            V::String(s) => {
                self.byte(0)?;
                self.blob(s.as_bytes())?;
            }
            V::Number(n) => {
                self.byte(1)?;
                self.blob(n.to_string().as_bytes())?;
            }
            V::Binary(b) => {
                self.byte(2)?;
                self.blob(b)?;
            }
            V::Bool(b) => {
                self.byte(3)?;
                self.byte(u8::from(*b))?;
            }
            V::Null => self.byte(4)?,
            V::List(values) => {
                if depth >= 32 {
                    return Err(BuildError::InvalidCursor);
                }
                self.byte(5)?;
                self.count(values.len())?;
                for v in values {
                    self.value(v, depth + 1)?;
                }
            }
            V::Map(values) => {
                if depth >= 32 {
                    return Err(BuildError::InvalidCursor);
                }
                self.byte(6)?;
                self.count(values.len())?;
                for (k, v) in values {
                    self.blob(k.as_bytes())?;
                    self.value(v, depth + 1)?;
                }
            }
            V::Strings(values) => {
                self.byte(7)?;
                self.count(values.len())?;
                for v in values {
                    self.blob(v.as_bytes())?;
                }
            }
            V::Numbers(values) => {
                self.byte(8)?;
                self.count(values.len())?;
                for v in values {
                    self.blob(v.to_string().as_bytes())?;
                }
            }
            V::Binaries(values) => {
                self.byte(9)?;
                self.count(values.len())?;
                for v in values {
                    self.blob(v)?;
                }
            }
        }
        Ok(())
    }
}
