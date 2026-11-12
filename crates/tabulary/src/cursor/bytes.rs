//! V1: version byte, binding, then sorted scalar key entries. All lengths/counts
//! are big-endian u32; schema positions are u64. Tags below are wire constants.
use super::*;
use crate::{DynamoType, FieldAccess, IndexKind, IndexProjection, KeyDefinition, Presence};
use aws_sdk_dynamodb::primitives::Blob;

const MAX: usize = 4 * 1024 * 1024;
// Bound work on untrusted numeric text, not an endpoint's formatting choice.
// Adjusted exponents are -130..=125 with at most 38 significant digits.
// Negative positional worst cases: 1 + 126 = 127 bytes for a large integer;
// 1 + 2 ("0.") + 129 zeroes + 38 digits = 170 for a small fraction.
// Round up to 192 bytes, leaving 22 bytes of spelling margin. Both cursor
// construction and the byte parser apply this cap before Decimal parsing.
pub(super) const MAX_NUMBER_KEY_BYTES: usize = 192;
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

/// Malformed cursor bytes. Diagnostics never retain token contents.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorError {
    TooLarge,
    UnknownVersion { version: u8 },
    Truncated,
    TrailingBytes,
    InvalidFormat,
    InvalidKey,
    DepthLimit,
}
impl fmt::Display for CursorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for CursorError {}

impl<R, I> Cursor<R, I> {
    /// Encode V1 bytes. Keys and filter constants are included without encryption.
    pub fn to_bytes(&self) -> Result<Vec<u8>, BuildError> {
        let mut w = Writer {
            bytes: Some(Vec::new()),
            len: 0,
        };
        w.byte(1)?;
        match &self.binding {
            Identity::Memory(binding) => w.binding(binding)?,
            Identity::Encoded(bytes) => w.raw(bytes)?,
        }
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

    /// Validate the entire token before allocating owned keys or binding data.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CursorError> {
        if bytes.len() > MAX {
            return Err(CursorError::TooLarge);
        }
        let mut r = Reader(bytes);
        let version = r.byte()?;
        if version != 1 {
            return Err(CursorError::UnknownVersion { version });
        }
        let start = r.0;
        let (fields, primary, index) = r.binding()?;
        let binding = &start[..start.len() - r.0.len()];
        let positions = [
            (Some(primary.0), 2048),
            (primary.1, 1024),
            (index.map(|k| k.0), 2048),
            (index.and_then(|k| k.1), 1024),
        ];
        let mut required = [None; 4];
        let mut f = Reader(fields);
        let fields = f.count(15)?;
        if positions
            .iter()
            .any(|(p, _)| p.is_some_and(|p| p >= fields as u64))
        {
            return Err(CursorError::InvalidKey);
        }
        for at in 0..fields {
            let (name, wire) = f.field()?;
            for (slot, (position, max)) in required.iter_mut().zip(positions) {
                if position == Some(at as u64) {
                    *slot = Some((name, wire, max));
                }
            }
        }
        let key_start = r.0;
        let count = r.count(6)?;
        if count == 0 || count > 4 {
            return Err(CursorError::InvalidKey);
        }
        let mut seen = [false; 4];
        let mut previous = None;
        for _ in 0..count {
            let name = r.text()?;
            if name.is_empty() || previous.is_some_and(|p| p >= name) {
                return Err(CursorError::InvalidKey);
            }
            previous = Some(name);
            let (tag, value) = r.scalar()?;
            let mut matched = false;
            for (slot, seen) in required.iter().zip(&mut seen) {
                if let Some((expected, wire, max)) = slot
                    && name == *expected
                {
                    if tag != *wire || (tag != 1 && value.len() > *max) {
                        return Err(CursorError::InvalidKey);
                    }
                    *seen = true;
                    matched = true;
                }
            }
            if !matched {
                return Err(CursorError::InvalidKey);
            }
        }
        if !r.0.is_empty() {
            return Err(CursorError::TrailingBytes);
        }
        if required
            .iter()
            .zip(seen)
            .any(|(slot, seen)| slot.is_some() && !seen)
        {
            return Err(CursorError::InvalidKey);
        }
        let mut k = Reader(key_start);
        k.count(6)?;
        let mut key = Item::new();
        for _ in 0..count {
            let name = k.text()?.to_owned();
            // The borrowed pass validated these bytes; do not reparse numbers.
            let tag = k.byte()?;
            let value = k.blob()?;
            let value = match tag {
                0 => AttributeValue::S(std::str::from_utf8(value).unwrap().into()),
                1 => AttributeValue::N(std::str::from_utf8(value).unwrap().into()),
                _ => AttributeValue::B(Blob::new(value)),
            };
            key.insert(name, value);
        }
        Ok(Self {
            key,
            binding: Identity::Encoded(binding.to_vec()),
        })
    }
}

pub(super) fn binding_size<R, I>(binding: &CursorBinding<R, I>) -> Result<(), BuildError> {
    let mut w = Writer {
        bytes: None,
        len: 5,
    };
    w.binding(binding)
}
pub(super) fn binding_bytes<R, I>(binding: &CursorBinding<R, I>) -> Result<Vec<u8>, BuildError> {
    let mut w = Writer {
        bytes: Some(Vec::new()),
        len: 0,
    };
    w.binding(binding)?;
    Ok(w.bytes.unwrap())
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

type Keys = (u64, Option<u64>);
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CursorError> {
        let out = self.0.get(..n).ok_or(CursorError::Truncated)?;
        self.0 = &self.0[n..];
        Ok(out)
    }
    fn byte(&mut self) -> Result<u8, CursorError> {
        Ok(self.take(1)?[0])
    }
    fn tag(&mut self, max: u8) -> Result<u8, CursorError> {
        let tag = self.byte()?;
        if tag > max {
            return Err(CursorError::InvalidFormat);
        }
        Ok(tag)
    }
    fn count(&mut self, min: usize) -> Result<usize, CursorError> {
        let n = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        if n > MAX / min {
            return Err(CursorError::TooLarge);
        }
        if n > self.0.len() / min {
            return Err(CursorError::Truncated);
        }
        Ok(n)
    }
    fn blob(&mut self) -> Result<&'a [u8], CursorError> {
        let n = self.count(1)?;
        self.take(n)
    }
    fn text(&mut self) -> Result<&'a str, CursorError> {
        std::str::from_utf8(self.blob()?).map_err(|_| CursorError::InvalidFormat)
    }
    fn position(&mut self) -> Result<u64, CursorError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn optional_position(&mut self) -> Result<Option<u64>, CursorError> {
        if self.tag(1)? == 1 {
            Ok(Some(self.position()?))
        } else {
            Ok(None)
        }
    }
    fn keys(&mut self) -> Result<Keys, CursorError> {
        Ok((self.position()?, self.optional_position()?))
    }
    fn field(&mut self) -> Result<(&'a str, u8), CursorError> {
        let name = self.text()?;
        self.position()?;
        self.tag(1)?;
        self.tag(4)?;
        Ok((name, self.tag(10)?))
    }
    fn names(&mut self) -> Result<(), CursorError> {
        for _ in 0..self.count(4)? {
            self.text()?;
        }
        Ok(())
    }
    fn index(&mut self) -> Result<Keys, CursorError> {
        self.text()?;
        self.tag(1)?;
        let keys = self.keys()?;
        if self.tag(2)? == 2 {
            self.names()?;
        }
        Ok(keys)
    }
    fn binding(&mut self) -> Result<(&'a [u8], Keys, Option<Keys>), CursorError> {
        self.text()?;
        self.text()?;
        let fields = self.0;
        for _ in 0..self.count(15)? {
            self.field()?;
        }
        let primary = self.keys()?;
        for _ in 0..self.count(15)? {
            self.index()?;
        }
        self.optional_position()?;
        let index = if self.tag(1)? == 1 {
            Some(self.index()?)
        } else {
            None
        };
        match self.byte()? {
            0 => {
                self.node(0)?;
                self.tag(1)?;
            }
            1 => {
                if self.tag(1)? == 1 {
                    let segment = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
                    let total = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
                    if total == 0 || total > 1_000_000 || segment >= total {
                        return Err(CursorError::InvalidFormat);
                    }
                }
            }
            _ => return Err(CursorError::InvalidFormat),
        }
        if self.tag(1)? == 1 {
            self.node(0)?;
        }
        if self.tag(1)? == 1 {
            self.names()?;
        }
        self.tag(1)?;
        Ok((fields, primary, index))
    }
    fn node(&mut self, depth: usize) -> Result<(), CursorError> {
        if depth > 32 {
            return Err(CursorError::DepthLimit);
        }
        match self.tag(3)? {
            0 => {
                self.tag(11)?;
                for _ in 0..self.count(1)? {
                    if self.tag(2)? == 2 {
                        self.value(0)?;
                    } else {
                        self.field()?;
                    }
                }
            }
            1 | 2 => {
                self.node(depth + 1)?;
                self.node(depth + 1)?;
            }
            _ => self.node(depth + 1)?,
        }
        Ok(())
    }
    fn scalar(&mut self) -> Result<(u8, &'a [u8]), CursorError> {
        let tag = self.tag(2)?;
        let value = self.blob()?;
        if tag == 1 {
            if value.len() > MAX_NUMBER_KEY_BYTES {
                return Err(CursorError::InvalidKey);
            }
            std::str::from_utf8(value)
                .ok()
                .and_then(|n| n.parse::<Decimal>().ok())
                .ok_or(CursorError::InvalidKey)?;
        } else if value.is_empty()
            || value.len() > 2048
            || (tag == 0 && std::str::from_utf8(value).is_err())
        {
            return Err(CursorError::InvalidKey);
        }
        Ok((tag, value))
    }
    fn number(&mut self) -> Result<Decimal, CursorError> {
        let text = self.text()?;
        let number: Decimal = text.parse().map_err(|_| CursorError::InvalidFormat)?;
        if number.to_string() != text {
            return Err(CursorError::InvalidFormat);
        }
        Ok(number)
    }
    fn value(&mut self, depth: usize) -> Result<(), CursorError> {
        match self.tag(9)? {
            0 => {
                self.text()?;
            }
            1 => {
                self.number()?;
            }
            2 => {
                self.blob()?;
            }
            3 => {
                self.tag(1)?;
            }
            4 => (),
            tag @ (5 | 6) => {
                if depth >= 32 {
                    return Err(CursorError::DepthLimit);
                }
                let mut previous = None;
                for _ in 0..self.count(if tag == 5 { 1 } else { 5 })? {
                    if tag == 6 {
                        let name = self.text()?;
                        if previous.is_some_and(|p| p >= name) {
                            return Err(CursorError::InvalidFormat);
                        }
                        previous = Some(name);
                    }
                    self.value(depth + 1)?;
                }
            }
            tag => {
                let count = self.count(4)?;
                if count == 0 {
                    return Err(CursorError::InvalidFormat);
                }
                let (mut previous, mut number) = (None, None);
                for _ in 0..count {
                    if tag == 8 {
                        let n = self.number()?;
                        if number.is_some_and(|p| p >= n) {
                            return Err(CursorError::InvalidFormat);
                        }
                        number = Some(n);
                    } else {
                        let v = if tag == 7 {
                            self.text()?.as_bytes()
                        } else {
                            self.blob()?
                        };
                        if previous.is_some_and(|p| p >= v) {
                            return Err(CursorError::InvalidFormat);
                        }
                        previous = Some(v);
                    }
                }
            }
        }
        Ok(())
    }
}
