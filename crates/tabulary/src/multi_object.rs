//! Closed sets of record types that share one table's physical keys.
use aws_sdk_dynamodb::types::AttributeValue;

use crate::{
    DecodeContext, DecodeError, DecodeFault, Document, DynamoType, FieldDescriptor, Item,
    KeyPattern, KeyPatterns, PathSegment, Record, RecordSchema,
};

/// One declared member: its variant name, record name, and key discriminators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultiVariant {
    pub name: &'static str,
    pub record: &'static str,
    pub keys: KeyPatterns,
}

/// A closed enum of the record types one table may hold.
///
/// `SCHEMA` is keys-only: its fields are exactly the shared physical partition
/// key and optional sort key, with no indexes or TTL. Each variant's key
/// patterns must be pairwise disjoint, so a stored key selects at most one
/// member. Manual implementations are supported; assert
/// [`multi_object_fault`] and [`member_fault`] in `const` items to get the
/// derive's compile-time checks.
pub trait MultiObject: Sized {
    const SCHEMA: RecordSchema;
    const VARIANTS: &'static [MultiVariant];
    /// A per-member proof type that only the enum's declaring module can
    /// construct, such as `pub struct Seal<R>(PhantomData<R>);` with a private
    /// field. `MemberOf` for `R` must produce a `MemberSeal<R>`; a real member's
    /// seal has a different type, so no other module or crate can add a member.
    type MemberSeal<R>;

    /// Decode `item` strictly as the member at `variant`, using `cx` for paths.
    fn decode_variant(
        variant: usize,
        item: &Item,
        cx: &mut DecodeContext,
    ) -> Result<Self, DecodeError>;
}

/// A record declared as one variant of `M`. Operations on a `MultiTable<M>`
/// accept only member types, so an undeclared record cannot reach that table.
/// Implementing it requires constructing `M::MemberSeal`, which only the enum's
/// declaring module can do; the derive's seal is also unnameable there.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a declared member of multi-object table `{M}`",
    note = "add a variant holding `{Self}` to `{M}`, or use a table bound to `{Self}`"
)]
pub trait MemberOf<M: MultiObject>: Record + Document {
    /// Position of this member's entry in `M::VARIANTS`.
    const VARIANT: usize;
    /// Proof that the enum's declaring module wrote this membership.
    const SEAL: M::MemberSeal<Self>;
    fn into_object(self) -> M;
}

/// Dispatch on the stored keys, then decode the selected member strictly.
///
/// A key that matches no member is `UnknownVariant` at that key. An invalid
/// manual implementation (see [`multi_object_fault`]) is refused before any
/// dispatch, so disjoint patterns select at most one member. The root record
/// of every error is the enum; its document names the selected member.
pub fn decode_object<M: MultiObject>(item: &Item) -> Result<M, DecodeError> {
    let cx = DecodeContext::new(M::SCHEMA.name);
    let index = select_variant::<M>(item, &cx)?;
    M::decode_variant(index, item, &mut cx.in_document(M::VARIANTS[index].record))
}

/// The position in `M::VARIANTS` that `item`'s stored keys select, with the
/// same refusals as [`decode_object`] and no member decode.
pub(crate) fn select_variant<M: MultiObject>(
    item: &Item,
    cx: &DecodeContext,
) -> Result<usize, DecodeError> {
    if let Some(code) = multi_object_fault::<M>() {
        return Err(cx.error(DecodeFault::Custom { code }));
    }
    let partition = key_text::<M>(item, Some(M::SCHEMA.primary.partition), cx, |k| k.partition)?;
    let sort = key_text::<M>(item, M::SCHEMA.primary.sort, cx, |k| k.sort)?;
    let by_partition = |v: &&MultiVariant| matches(v.keys.partition, partition);
    let selected = M::VARIANTS
        .iter()
        .enumerate()
        .find(|(_, v)| by_partition(v) && matches(v.keys.sort, sort));
    let Some((index, _)) = selected else {
        // Name the key that failed to discriminate: the sort key when the
        // partition alone matched a member, otherwise the partition key.
        let field = match M::SCHEMA.primary.sort {
            Some(sort) if M::VARIANTS.iter().any(|v| by_partition(&v)) => sort,
            _ => M::SCHEMA.primary.partition,
        };
        return Err(at::<M>(cx, field).error(DecodeFault::UnknownVariant));
    };
    Ok(index)
}

fn matches(pattern: KeyPattern, value: Option<&str>) -> bool {
    match value {
        Some(value) => pattern.matches(value),
        None => pattern == KeyPattern::Any,
    }
}

fn at<M: MultiObject>(cx: &DecodeContext, position: usize) -> DecodeContext {
    cx.at(PathSegment::Attribute(
        M::SCHEMA.fields[position].name.into(),
    ))
}

fn key_text<'a, M: MultiObject>(
    item: &'a Item,
    position: Option<usize>,
    cx: &DecodeContext,
    pattern: fn(&KeyPatterns) -> KeyPattern,
) -> Result<Option<&'a str>, DecodeError> {
    let Some(position) = position else {
        return Ok(None);
    };
    let field = &M::SCHEMA.fields[position];
    let cx = cx.at(PathSegment::Attribute(field.name.into()));
    let value = item
        .get(field.name)
        .ok_or_else(|| cx.error(DecodeFault::Missing))?;
    if M::VARIANTS
        .iter()
        .all(|v| pattern(&v.keys) == KeyPattern::Any)
    {
        return Ok(None);
    }
    match value {
        AttributeValue::S(text) => Ok(Some(text)),
        other => Err(crate::value::wrong_type(other, DynamoType::S, &cx)),
    }
}

/// Validate a keys-only schema and pairwise-disjoint discriminators.
/// Returns a stable fault code; derives assert `None` at compile time.
pub const fn multi_object_fault<M: MultiObject>() -> Option<&'static str> {
    let schema = M::SCHEMA;
    let keys = schema.primary;
    let expected = if keys.sort.is_some() { 2 } else { 1 };
    if schema.name.is_empty() {
        return Some("empty_multi_object_name");
    }
    if schema.fields.len() != expected
        || !schema.indexes.is_empty()
        || schema.ttl.is_some()
        || !key_field(schema.fields, keys.partition)
    {
        return Some("multi_object_keys_only_schema");
    }
    if let Some(sort) = keys.sort
        && (sort == keys.partition
            || !key_field(schema.fields, sort)
            || crate::key_pattern::same(
                schema.fields[sort].name,
                schema.fields[keys.partition].name,
            ))
    {
        return Some("multi_object_keys_only_schema");
    }
    let variants = M::VARIANTS;
    if variants.is_empty() {
        return Some("empty_multi_object");
    }
    let mut i = 0;
    while i < variants.len() {
        let v = variants[i].keys;
        if !pattern_ok(v.partition, schema.fields[keys.partition].wire) {
            return Some("invalid_variant_key_pattern");
        }
        match keys.sort {
            Some(sort) if !pattern_ok(v.sort, schema.fields[sort].wire) => {
                return Some("invalid_variant_key_pattern");
            }
            None if !matches!(v.sort, KeyPattern::Any) => {
                return Some("invalid_variant_key_pattern");
            }
            _ => {}
        }
        let mut j = 0;
        while j < i {
            let w = variants[j].keys;
            if v.partition.overlaps(w.partition) && v.sort.overlaps(w.sort) {
                return Some("overlapping_variant_keys");
            }
            j += 1;
        }
        i += 1;
    }
    None
}

/// Validate that `R` is the record its declared variant names, that it shares
/// `M`'s physical keys, and that its key templates always produce text matching
/// that variant's discriminators. Every `MultiTable` member operation asserts
/// this at compile time. `MemberSeal` keeps other modules from adding members;
/// this check catches a mistaken membership inside the declaring module.
pub const fn member_fault<M: MultiObject, R: MemberOf<M>>() -> Option<&'static str> {
    let (table, member) = (M::SCHEMA, R::SCHEMA);
    if R::VARIANT >= M::VARIANTS.len() {
        return Some("unknown_member_variant");
    }
    if !same_key(
        table,
        member,
        Some(table.primary.partition),
        Some(member.primary.partition),
    ) {
        return Some("member_partition_key_mismatch");
    }
    if !same_key(table, member, table.primary.sort, member.primary.sort) {
        return Some("member_sort_key_mismatch");
    }
    if !crate::key_pattern::same(M::VARIANTS[R::VARIANT].record, <R as Document>::NAME) {
        return Some("member_record_mismatch");
    }
    let declared = M::VARIANTS[R::VARIANT].keys;
    if !R::KEY_PATTERNS.partition.implies(declared.partition) {
        return Some("member_partition_pattern_mismatch");
    }
    if !R::KEY_PATTERNS.sort.implies(declared.sort) {
        return Some("member_sort_pattern_mismatch");
    }
    None
}

const fn key_field(fields: &[FieldDescriptor], position: usize) -> bool {
    position < fields.len()
        && fields[position].index == position
        && !fields[position].name.is_empty()
        && matches!(
            fields[position].wire,
            DynamoType::S | DynamoType::N | DynamoType::B
        )
}

const fn pattern_ok(pattern: KeyPattern, wire: DynamoType) -> bool {
    match pattern {
        KeyPattern::Any => true,
        KeyPattern::Prefix(text) | KeyPattern::Exact(text) => {
            !text.is_empty() && matches!(wire, DynamoType::S)
        }
    }
}

const fn same_key(
    table: RecordSchema,
    member: RecordSchema,
    a: Option<usize>,
    b: Option<usize>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a < table.fields.len()
                && b < member.fields.len()
                && crate::key_pattern::same(table.fields[a].name, member.fields[b].name)
                && wire_eq(table.fields[a].wire, member.fields[b].wire)
        }
        _ => false,
    }
}

const fn wire_eq(a: DynamoType, b: DynamoType) -> bool {
    matches!(
        (a, b),
        (DynamoType::S, DynamoType::S)
            | (DynamoType::N, DynamoType::N)
            | (DynamoType::B, DynamoType::B)
    )
}
