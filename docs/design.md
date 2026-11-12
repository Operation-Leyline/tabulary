# tabulary design

Status: **implemented library with documented contract gaps**. Record codecs and
derives, typed expressions, prepared operations, cursor bytes, batches,
transactions, and table definitions have unit, compile-fail, and SDK-fixture
coverage. CI runs the SDK/MSRV feature matrix and the discovered endpoint suite
against a pinned dynoxide emulator. All seven [examples](examples/README.md)
compile as real test modules and their public functions have result assertions.
Section 16 lists remaining API and real-service verification gaps; passing these
tests does not establish exact DynamoDB equivalence.

## 1. Scope and design rules

One Rust value represents one DynamoDB item. Its declaration owns the wire names,
key schema, converters, and decode rules. Operations name the record and a table
at runtime. A table binding contains a name and schema, never an AWS client.

The runtime prepares AWS SDK inputs and decodes SDK results. It has no async
runtime, transport, clock, credentials, retry policy, or `send` method.
`request.apply(client.operation()).send().await` is always application code.
An application can inspect the entire input before it sends it.

The library does not plan queries, migrate schemas, infer access patterns, or
require single-table modeling. Different records may bind to the same table only
when their physical key and index schemas agree. It does not create registries,
inject entity tags, or decide tenancy and authorization.

All examples use invented retail and delivery models. The public design includes
no application-specific schema or workload measurements.

### 1.1 Prior art and what changes here

The comparisons below describe the linked documentation, checked while preparing
this design. They are API comparisons, not claims that alternatives are unusable.

| Reference | Take | Choose differently |
| --- | --- | --- |
| [serde_dynamo 4.3.0](https://docs.rs/serde_dynamo/4.3.0/serde_dynamo/) | Direct struct/item conversion, explicit binary and set representations, support for rich Rust shapes. | Do not use a general Serde walk as the primary schema. Generate field access and path-aware errors, and derive expressions from the same schema. No intermediate JSON or implicit defaults. |
| [modyne 0.3.0](https://docs.rs/modyne/0.3.0/modyne/) | Typed key conditions, index metadata, and read-only projection types. | Do not require its single-table entity/aggregate model or a table trait that supplies a client. Table binding is independent of execution. |
| [dynamodb-expression](https://docs.rs/dynamodb-expression/0.2.0-beta.11/dynamodb_expression/) | Expression ASTs, aliases, and inspectable name/value maps. | Paths come from record handles rather than parsed attribute strings; operands also carry record and wire-type constraints. |
| [raiden-dynamo README](https://github.com/raiden-rs/raiden-dynamo/blob/master/README.md) | Derive-based field naming and convenient typed operation builders. | Do not generate clients or execute/retry operations. Own structured fault kinds and complete nested paths. Avoid another required builder framework. |

The current raiden-dynamo README already documents an attribute conversion error
with an attribute name and a source error. Therefore, “it has no typed decode
errors” is too broad. The distinction here is a specified path and fault taxonomy
that also covers composite keys, variants, and old items in failed writes.

AWS SDK feature requests [#70](https://github.com/awslabs/aws-sdk-rust/issues/70),
[#1172](https://github.com/awslabs/aws-sdk-rust/issues/1172), and
[#1355](https://github.com/awslabs/aws-sdk-rust/issues/1355) explain the demand for
struct conversion. tabulary does not depend on an SDK roadmap commitment.

### 1.2 Crates and supported toolchain

- `tabulary`: codecs, schema metadata, handles, expressions, prepared operations,
  errors, cursors, batch state, and table definitions.
- `tabulary-derive`: `Record`, `Document`, `StringEnum`, `Variants`, and
  `Projection` derives, re-exported by the runtime's `derive` feature.

Both use edition 2024, MSRV **1.97.1**, Apache-2.0, and
`#![forbid(unsafe_code)]`. The runtime uses `aws-sdk-dynamodb = "1.130.0"`
with default features disabled, plus the optional derive crate. The runtime's default feature set is
`["derive"]`; disabling it supports manual trait implementations.
The application enables SDK transport features.
The macro crate needs only `syn`, `quote`, and `proc-macro2`.
There is no mandatory Serde, decimal, timestamp, async-trait, or error framework.
Test dependencies may include `trybuild`, `tokio`, and `aws-config`.

The SDK API floor is **aws-sdk-dynamodb 1.130.0**, whose modeled conditional
failure item is linked in §11.1. The first SDK PR records
`version = "1.130.0", default-features = false` and commits the resolved
lockfile. CI tests the exact floor and a current MSRV-compatible 1.x release.
Supporting the whole historical 1.x series is not promised. Dependency updates
must pass MSRV CI; a compiler requirement change is an announced release decision,
not an accidental lockfile update. A floor compile failure must be resolved and
documented in that PR before SDK-dependent work lands.

## 2. Record declarations and generated names

```rust,ignore
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", unknown = "deny")]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    line_no: u64,
    quantity: u64,
    #[tabulary(rename = "giftNote")]
    gift_note: Option<String>,
}
```

A named struct is a record; tuple structs and unions are rejected. Every ordinary
field is one attribute, with two explicit exceptions: a flattened `Variants`
field occupies its declared tag and payload attributes, and composite keys add
declared synthesized attributes. No field is silently skipped.

Each field must have an explicit `rename` or inherit a container `rename_all`.
Supported rules are `snake_case`, `camelCase`, `PascalCase`, and
`SCREAMING_SNAKE_CASE`. Explicit rename wins. Duplicate wire names, including
synthetic keys, tags, and variant payloads that overlap common fields, fail the
derive at the offending field. Empty names are rejected.

The derive emits peer types `OrderAttrs`, `OrderKey`, `OrderPrimary`,
and `OrderIndexes`.
Each field has a constant handle, such as `OrderAttrs::QUANTITY`.
`OrderAttrs::GIFT_NOTE.name()` returns its declared wire name. Access patterns
never repeat wire names as string literals. Generated peer visibility matches
the record; each handle is no more visible than its source field.
Generated names that collide with existing Rust items are ordinary compiler
errors; `#[tabulary(generated_prefix = "Purchase")]` changes the prefix explicitly.

`Attr<R, T, Presence, Access, Codec>` is a copyable, zero-data-type-parameter
handle with a static descriptor. `T` is the logical value type; for an optional
field it is the inner type. Presence is `Required` or `Optional`; access is
`Mutable`, `KeyOnly`, `VersionOnly`, `IndexSourceOnly`, or `DerivedOnly`.
The codec determines its DynamoDB wire capabilities. The default type parameters
are `Required, Mutable, Native<T>`. A required number handle thus has type
`Attr<Order, u64>`. Optional values use `set(T)`, comparisons against `T`,
and `remove()`; callers never encode “absent” as an expression value.

Milestone 10 introduces the metadata-only `Record { const SCHEMA: RecordSchema; }`
contract; the document/key supertraits and methods sketched below arrive with
their owning milestones. `RecordSchema` initially contains `name`, static
`fields`, and a primary `KeyDefinition` of partition/sort field positions.
`FieldDescriptor` contains the wire name, declaration index, presence, access,
and codec's DynamoDB category. Index, TTL, and version schema metadata arrive
with their owning milestones. The doc-hidden `field(name, index)`,
`keys(partition, sort)`, and three-argument `schema(name, fields, primary)`
helpers construct this initial surface; §15 sketches the completed surface.
Presence and access marker traits are sealed; custom codecs still implement
the public `Converter` trait. `Attr::validate()` checks all descriptor metadata
against its owner's field at the declared position. Mutable handles additionally
expose `validate_mutation()`, which rejects physical primary-key positions even
if manual schema metadata labels them mutable. Future expression and operation
builders must call these checks before accepting descriptors; these methods
alone do not construct expressions or perform value conversion.

The user constructs records with normal Rust struct literals. No generated
record builder is required, and the derive never generates `Default`.

Primary and index schema handles are generated concrete peer types, not inherent
implementations on a foreign generic runtime type. A primary schema implements
`PrimarySchema<R>` with logical Partition, PartitionCodec, Sort, and a physical
KeyDefinition. Sort is `NoSort` or `SortKey<T, Codec>`. Secondary handles
implement the corresponding `IndexSchema<R>` contract. Their generated inherent
`partition` methods accept one argument per logical component and pack them into
the associated partition type; composite codecs format that tuple. The runtime's
`KeyCondition<R, I>` exposes valid sort methods through I's Sort type.
Query cursors use the concrete primary peer, such as `OrderPrimary`.
The earlier `Primary<R>` alias sketch is not implemented because `Record`
remains metadata-only. This avoids Rust's foreign inherent-impl restriction.

### 2.1 Runtime traits

These are the current runtime trait boundaries. `Record` owns metadata,
`table_definition()`, and `billed_size()` (the last requires `Document`).
It has no document supertrait or associated key/primary types.
The derive emits inherent `to_item`, `from_item`, `key`, and `primary` methods.
Manual records implement `Document` and `RecordKey<R>` separately when needed.
Manual codec, document, and record implementations remain supported without
`derive`; typed handles, key-condition entry points, and projections are generated
by derives only. Their reserved construction hooks are not public API.
Use the generated primary peer as a query cursor marker; a table scan uses `()`.
There is no `Primary<R>` alias; use the concrete primary peer.

```rust,ignore
pub type Item = std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>;

pub trait Document: Sized {
    const FIELDS: &'static [FieldDescriptor] = &[];
    const NAME: &'static str;
    fn encode_map(&self, cx: &mut EncodeContext) -> Result<Item, EncodeError>;
    fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError>;
}

pub trait Record: Sized {
    const SCHEMA: RecordSchema;
    fn billed_size(&self) -> Result<BilledSize, EncodeError> where Self: Document;
    fn table_definition() -> TableDefinition<Self>; // provided default
}

pub trait Value: Sized {
    type Wire: WireType;
    fn encode_value(&self, cx: &mut EncodeContext)
        -> Result<aws_sdk_dynamodb::types::AttributeValue, EncodeError>;
    fn decode_value(value: &aws_sdk_dynamodb::types::AttributeValue, cx: &mut DecodeContext)
        -> Result<Self, DecodeError>;
}

pub trait RecordKey<R: Record>: Sized {
    fn encode_key(&self) -> Result<Item, EncodeError>;
    fn decode_key(item: &Item) -> Result<Self, DecodeError>;
}
```

`Record` derives also emit `Document` and `Value` implementations, allowing a
record to be embedded as an `M`. `Document` derives omit keys and table
operations. Generated functions use direct field access, static descriptors, and
monomorphized codecs. There is no reflection, registration, `Any`, type
downcast, or run-time schema discovery.

Contexts track root record, current document, wire attribute segments, and list
indexes. Read helpers borrow the item; they never convert it through JSON.
Manual `Converter`, `Value`, `Document`, `Record`, and `RecordKey<R>`
implementations are supported. Typed attribute handles, key conditions, and
projections (including `ProjectionSource`) are produced by derives only;
manual construction of those typed interfaces is outside the supported API.
Reserved `__private` macro helpers are not public API and carry no semver guarantee.

`WireType` exposes `const DYNAMO_TYPE: DynamoType`. The public `wire` module
provides zero-sized `String`, `Number`, `Binary`, `Bool`, `Null`, `List`, `Map`,
`StringSet`, `NumberSet`, and `BinarySet` markers for the corresponding SDK tags.
Markers describe a codec's category; they do not implement value conversion.

## 3. Keys and secondary indexes

Exactly one partition key and at most one sort key are required.
`#[tabulary(partition_key)]` and `#[tabulary(sort_key)]` mark physical fields.
Key wire types must be `S`, `N`, or `B`; primary keys cannot be optional,
null, empty strings, or empty binary values. Key constructors validate the
service byte limits and numeric domain before building a request.
`OrderKey::new(order_ref, line_no)` returns `Result<OrderKey, EncodeError>`.
It contains owned logical key inputs and validates again when encoded.

### 3.1 Composite key templates and inverse parsing

```rust,ignore
#[derive(tabulary::Record)]
#[tabulary(rename_all = "camelCase")]
#[tabulary(partition(name = "parcelKey",
    template = "parcel!{shop_code:hex}!{parcel_token:hex}"))]
struct Parcel {
    shop_code: String,
    parcel_token: [u8; 16],
    #[tabulary(sort_key)]
    parcel_no: u64,
    assigned_courier: Option<String>,
}
```

`ParcelKey::new(shop_code, parcel_token, parcel_no)` uses the template.
`ParcelAttrs::PARTITION` exposes the synthesized key; field keys retain their
own named handles. Template output is an `S`. Each referenced source remains
stored under its own declared attribute: no implicit elision or double meaning.
Changing a composite source in place is forbidden, as with a primary key field.
Moving keys requires an explicit delete and put, optionally in one transaction.

The template language has literal UTF-8 text, escaped braces `{{` and `}}`,
and `{field:codec}` components. Built-in component codecs are:

| Codec | Inputs | Canonical form |
| --- | --- | --- |
| `hex` | UTF-8 string or bytes | Lowercase hex, even length; fixed arrays enforce length; string inverse checks UTF-8. |
| `dec` | Integers | Canonical decimal, optional minus only for signed types; no leading zero except zero. |
| `dec20` | `u64` | Exactly twenty decimal digits, preserving unsigned lexical order. |
| `text` | String | UTF-8 with template separators forbidden in the component. |

Milestone 15a supplies these runtime codecs in `key_component`, implementing
`KeyComponentCodec<T>` with `format(&T) -> Result<String, KeyFormatFault>` and
`parse(&str) -> Result<T, KeyFormatFault>`. Invalid spellings are structured
faults, without retaining input values. Milestone 15b generates typed partition
and sort-prefix carriers with private payload fields, complete-component tuple
implementations, and reserved text separator validation. Generated key expansion
and source comparison remain in 15b.
Custom template declaration syntax and delimiter metadata are unspecified and
deferred; the parser continues to recognize only the four built-in names.

Every codec is injective within its accepted domain and implements an inverse.
Adjacent variable-width components, ambiguous boundaries, duplicate component
references, unknown fields, and separators admitted by a variable component's
codec are compile errors. An entire-key single component needs no separator.
Custom components implement `KeyComponentCodec<T>` with parse and format;
the author declares reserved delimiters and tests round trips. They cannot
silently replace the built-in grammar.

Milestone 14 implements the built-in grammar and validation; milestone 15b adds
formatting, inverse parsing, and source/codec type bounds. Components name Rust fields (including unprefixed
raw identifiers), not renamed wire attributes. Primary sources cannot be
optional or have defaults. Nonempty literal-only templates are permitted.

For boundary validation, a run of adjacent components between literals may
contain at most one variable-width component. `dec20` has width 20; `hex` on
`[u8; N]` has width `2*N` when N is an integer literal. Other array length
expressions are conservatively variable-width at this stage. A literal after
a variable-width run must start outside the numeric/hex output alphabets of
every component in that run (decimal includes `-`). This conservative rule
also rejects overlapping multi-character delimiters such as `a!` after hex.
`text` instead reserves the first character of the following literal separator
in its accepted input domain, preventing overlapping delimiter matches too;
milestone 15 must reject source strings containing that character when formatting.
Fixed-width runs need no delimiter restriction. Escaped braces are decoded
before these checks, and literal prefixes require no separator check.

On decode, the generated parser reads each synthesized primary key, parses all
components, and checks them against their separately decoded source fields.
A missing component attribute is still `Missing`; a parsed component mismatch
is `KeyMismatch`, never a default or an ignored duplicate. Malformed prefixes,
hex, and lengths have `KeyFormat` faults at the physical key plus component
path. Decode never needs a caller-provided key to fill missing attributes.

Template helpers expose typed query prefixes only at complete component
boundaries, for example `Parcel::primary().partition(shop, token)`.
A composite sort key additionally generates `sort_prefix(parts...)` for proper
leading components. There is no raw string key-condition escape hatch.

For milestone 15b, `primary().partition(component_a, component_b)` and
`primary().sort_prefix((component_a,))` return generated `ParcelPartition` and
`ParcelSortPrefix` peer types, with private payload fields, explicit `as_str()`
access, and redacted Debug. Prefix arguments are tuples: a generated
`ParcelPrefixParts` trait covers each nonempty proper leading tuple, so several prefix
arities coexist without Rust method overloading. Prefixes include any literal
after the last supplied component. Empty tuples, partial encoded strings,
nonleading components, and the complete sort tuple have no generated implementation.
Boundaries inside a run that already contains a variable-width component are
excluded until a following literal is supplied; a missing fixed-width tail
cannot delimit a query prefix. Prefix traits return the concrete generated
carrier, so an external implementation cannot wrap arbitrary strings. As with
manual schemas, a record's own declaring module remains trusted.
Key-condition construction itself belongs to milestone 23; these staged
formatting helpers perform no query. Primary key constructor inputs follow
partition components then sort components, retaining a shared source only once.
Synthesized handles are `DerivedOnly`; their stored source handles are `KeyOnly`.

Milestone 23 promotes `partition(...)` on primary and index peers to an owned
`KeyCondition`; the previous composite formatting helper is named
`partition_value(...)` and retains its private typed carrier and `as_str()`.
Composite query entry points call this helper and retain any formatting error
until `build()`, matching field-keyed entry points. `sort_prefix` still returns
its typed carrier immediately; `sort_begins_with` accepts that carrier only,
never a raw string or a full component tuple. Other composite sort predicates
accept the complete logical tuple and use the existing template codec.

### 3.2 Index declarations, sparsity, and projections

```rust,ignore
#[tabulary(index(ident = ByCourier, name = "by_courier", kind = global,
    partition = assigned_courier, sort = parcel_no, sparse, project = all))]
```

The generated handle is `ParcelIndexes::BY_COURIER`, with a distinct schema
marker `ParcelByCourier`. `kind = local` requires the same partition key as
the base table and a distinct sort key. Hash-only GSIs are allowed. An index may
use `partition = field` / `sort = field` or the same nested
`partition(name = ..., template = ...)` / `sort(...)` syntax as primary keys.
Index names and identifiers are declared here only.

Milestone 16 is split at the field/template boundary to keep each pull request
within the milestone size limit. Milestone 16a provides field-keyed index metadata,
typed `IndexSchema<R>` peers, and literal index-key update validation.
It rejects composite index declarations until 16b. Milestone 16b reuses the primary
template formatter and inverse parser for required composite index inputs, and
adds physical-key `AttributeDefinition` extraction.
Composite index sources use `IndexSourceOnly`, except primary dependencies,
which remain `KeyOnly`. Synthetic handles use the index identifier plus
`PARTITION` or `SORT`, for example `ParcelAttrs::BY_COURIER_PARTITION`.
Identical templates with the same physical name share one descriptor; a
different template or an ordinary field with that name is a collision.
Milestone 17 adds `sparse` and optional index inputs with membership checks on
encode and full decode. Key-condition builders arrive in
milestone 23; index markers in milestone 16 do not perform queries.
Literal SET values and literal `if_not_exists` defaults targeting simple index
keys use the same key-value checks as record encoding. Results computed from
stored values cannot be validated locally and remain subject to service checks.

Non-sparse index inputs must be required. Sparse indexes require at least one
optional input. If any input is absent, omit all synthesized index attributes;
ordinary source fields are preserved. Reused physical fields may therefore be
partially present, which DynamoDB treats as a non-member. A present wrong-typed
or malformed input is an error, never a reason to skip an index. Partial
synthesized keys or keys inconsistent with present sources are corruption on
read. Sparse membership is computed on encode and checked on full decode.
Membership inputs are the ordinary fields used directly as index keys or as
template components. Inputs cannot have defaults. Each present template component
must satisfy its codec and separator rules even when another input is absent.
Conditional synthesized handles have `Optional` presence and `DerivedOnly` access;
their logical index codecs still accept the inner component types.

When identical templates share a physical attribute, emit that attribute if any
index using it has all inputs present, or if it is also a primary key. An absent
input in another index does not remove a key that an active index needs. Full
decode applies the same rule per physical attribute: a needed missing key is
`Missing`, an unneeded present key is `IndexMismatch`, and a parsed source
disagreement is `KeyMismatch`. This permits partial reused fields while rejecting
partial synthesized keys that no other index needs.

`project = all`, `project = keys_only`, and
`project = include(field_a, field_b)` describe index storage. Include entries
refer to Rust fields; the derive resolves their wire names. A sparse record
can still have required common fields when it is read from its base table.
Include lists reject primary and index keys, which DynamoDB projects automatically
([Projection API](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_Projection.html)).

Index reads have a separate `IndexTable<R, I>` type:

```rust,ignore
let idx = parcels.index(ParcelIndexes::BY_COURIER);
let request = idx.query(
    ParcelIndexes::BY_COURIER.partition("courier-lime".into()).sort_ge(8),
    tabulary::ReadConsistency::Eventual,
).build()?;
```

Strong reads on a GSI are rejected at build with `UnsupportedConsistency`.
LSIs and primary table reads accept explicit strong or eventual consistency.
For a non-ALL index, a full record read is rejected; require `select::<P>()`
and verify that every requested attribute is projected. No automatic follow-up
GetItem occurs. The first version applies this restriction to LSIs too, avoiding
implicit base-table fetches.

### 3.3 Updating fields that feed indexes

General `SET` and `REMOVE` on a source of a synthesized index key are
unavailable. Updating only a source would leave the index key stale.
`update.reindex(ParcelIndexes::BY_COURIER, inputs)` accepts the complete typed
input set for that index, emits source changes plus synthesized key changes,
and removes synthetic attributes when it becomes sparse. For reused field keys,
it emits the corresponding source updates directly.

The derive generates `ParcelByCourierInputs` with the index's logical input
fields. If several indexes depend on one field, a build validates that all
affected indexes have complete reindex input sets and agree on shared values.
Otherwise it returns `IncompleteReindex` or `ConflictingReindex`.
Primary key dependencies are immutable even through this API. Full-record puts
always recompute and validate all index keys.
If a complete index input tuple includes a primary-key component, it must equal
the operation's key and is not emitted as a SET target. Synthesized-index source
handles use `IndexSourceOnly`; simple field indexes need no extra protection
unless that field also feeds a synthesized key.

Milestone 32 generates `<Record><Index>Inputs`, retaining source field names,
types, presence and visibility. Its `new(...)` takes each distinct logical input
in partition-then-sort component order, including explicit `None` inputs. Generic
inputs use this constructor to initialize their private record marker. Encoding
is deferred until build and reuses record key formatting and sparse validation.
Reindex actions follow ordinary actions in schema field order; shared source
targets appear once, and a shared synthetic attribute is retained if any supplied
index needs it. All indexes sharing a mutable input or synthetic target must be
supplied, even when the new value might equal the stored value. Disagreement on
a shared source, or supplying an index twice, is `ConflictingReindex`.

Primary inputs are compared with logical values decoded from the operation key;
a mismatch is `OperationConstraint { code: "primary_source_mutation" }`. They
are never mutation targets. Standalone `Update::build()` has no operation key,
so a reindex containing primary inputs fails with `reindex_requires_key`; the
ordinary UpdateItem build supplies that key, including transaction conversion.
`ReindexSchema<R>` describes generated bindings using the same expansion path;
manual typed reindex bindings are outside the supported API.

## 4. Value mapping and validation

No numeric value passes through `f64` unless its declared Rust type is `f64`.
Codecs validate DynamoDB limits both on encode and decode.

| Rust type | DynamoDB representation and rule |
| --- | --- |
| `String` | `S`; empty allowed except keys. No coercion from another type. |
| `u64`, `i64` | `N`; exact integer conversion, checked range, no fractional truncation. Mathematically integral exponent forms are accepted. |
| `u128` | `N` when representable; reject values outside the service's precision domain with `NumberRange`. See the explicit full-range alternative below. |
| `f64` | `N` using a shortest round-trip decimal. Reject NaN, infinities, negative zero, and values outside the service number domain. |
| `Decimal` | `N`; a built-in canonical sign/coefficient/exponent decimal, exact mathematical value, checked significant digits and exponent. Parsing never rounds. Scale and spelling are not preserved. |
| `Vec<u8>`, `[u8; N]` | `B`; fixed arrays require exact length on read. Raw byte size, not base64 length. |
| `bool` | `BOOL`. |
| `StringSet`, `NumberSet<T>`, `BinarySet` | `SS`, `NS`, `BS`; validate nonempty, reject duplicate normalized members, sort canonical encoding for deterministic output. |
| `List<T>` | `L`; wrapper around `Vec<T>`, preserving order and empty lists. |
| `Map<T>` | `M`; wrapper around `BTreeMap<String, T>` for dynamic data keys. |
| A derived `Document` or `Record` | `M` when nested; its naming, validation, and unknown-field policy remain active. |
| `Option<T>` at a field | Absent means `None`; present means decode `T` strictly. Encode `None` as absent. `NULL` is a wrong type. |
| `Nullable<T>` | Explicit `NULL(true)` or `T`; required field unless wrapped in `Option`. `NULL(false)` is malformed. |
| A derived `StringEnum` | `S` with declared variant names. Unknown strings are `UnknownVariant`. |
| `TimestampSeconds`, `TimestampMillis`, `TimestampMicros` | `N` signed checked epoch units, with no unit guessing or clock access. |
| `TtlSeconds` | `N` unsigned whole epoch seconds. Optional TTL uses `Option<TtlSeconds>`. |
| `Secret<T>` | Same wire value as `T`; only explicit access reveals the inner value. |

`Vec<T>` is deliberately not a blanket list implementation: it would overlap
the required `Vec<u8>` binary mapping. Use `List<u8>` for numeric byte lists.
`Option<T>` is field presence, not a `Value` implementation; nullable list or
map elements must use `Nullable<T>`. Sets require supported scalar wire
categories; there is no set of arbitrary documents or floating NaNs.

`List::from(Vec<T>)` and `Map::from(BTreeMap<String, T>)` retain their public
tuple storage. Sets have private vector storage, `as_slice()` and `into_inner()`;
`TryFrom<Vec<_>>` validates members and returns `EncodeError` rooted at the
wrapper's type name. Encoding validates again in the caller's record context.
`NumberSet<T>` requires `T: Value<Wire = wire::Number>`. Numeric members normalize
through `Decimal`; canonical strings sort lexicographically, binary members by
raw bytes. Empty string and binary members are allowed; empty sets are not.
Set member errors include their input index. Lists preserve input order, maps
traverse keys in sorted order, and sets retain input order in memory but encode
in canonical order. The SDK's map remains a `HashMap`; its iteration order is
not a serialized ordering guarantee.

DynamoDB has a 38-significant-digit numeric domain and a bounded exponent range.
Some `u128` values require 39 significant digits, and some finite binary64
values are too small or large. The library cannot store all such values as
DynamoDB `N`. It must reject rather than round. The opt-in field converter
`tabulary::convert::u128_string` stores the complete `u128` range as decimal
`S`; its handle has string-wire equality and no numeric ADD or ordering
methods. This is an explicit schema choice.
The converter accepts only canonical unsigned decimal strings: `0` or ASCII
digits starting with `1` through `9`, within `u128` range. Signs, leading zeros,
whitespace, fractions, and exponent notation return
`Custom { code: "invalid_u128_string" }`. Other wire types return `WrongType`.
Its distinct `convert::u128_string::Wire` marker keeps equality separate from
native string ordering capabilities.
[AWS number limits](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/HowItWorks.NamingRulesDataTypes.html)
define the boundary tests.

On an `f64` read, parse finite binary64, then canonicalize its shortest decimal
and compare it numerically to the received decimal. If re-encoding would change
the stored decimal, return `InexactNumber`. This preserves accepted decimal
values through read/write and preserves accepted Rust finite nonzero values
through write/read; it cannot make arbitrary decimals exactly representable in
binary floating point. `Decimal` is the choice for exact decimal arithmetic.
`Decimal` implements FromStr, canonical Display, Eq, and Ord. A private u128
coefficient, signed exponent, and sign suffice for the supported precision;
arbitrary-precision arithmetic is not required for mapping.

The `f64` codec formats its shortest round-trip digits using `Decimal`'s
canonical Display rules below. Encoding returns `NonFiniteFloat` for NaN or
infinity, `NegativeZero` for negative zero, and `NumberRange` outside the service
domain. Decoding validates decimal syntax and the service domain first, with
`InvalidNumber` and `NumberRange` respectively. A negative-zero spelling returns
`NumberRange` rather than being normalized to positive zero. A valid service
decimal whose binary64 value would write a different decimal returns
`InexactNumber`, including rounding across the upper service boundary.

The decimal grammar is `[+-]?(digits+(\.digits*)?|\.digits+)([eE][+-]?digits+)?`,
using ASCII digits only, with no whitespace. Leading and trailing zeroes do not
count toward precision. Zero of either sign and any exponent normalizes to `0`.
Canonical Display uses plain positional notation whenever it needs at most
38 integer digits and at most 38 fractional digits. It has no leading zeroes
except a single `0` before a decimal point, and no trailing fractional zeroes:
`12.30` becomes `12.3`, `.5` becomes `0.5`, and `1.50e2` becomes `150`.
Outside those limits it writes the signed coefficient without leading or
trailing zeroes, followed by `e` and the exponent (`1e-130`, `1e125`).
Nonzero magnitude must be between `1e-130` and
`9.9999999999999999999999999999999999999e125`, inclusive.
`Decimal::from_str` returns `DecodeFault::InvalidNumber` for malformed syntax
and `DecodeFault::NumberRange` for precision or service-domain violations.
`DecodeFault` implements `Display` and `std::error::Error`, so parsing supports
`?` with boxed errors. Its display contains fault metadata, never the input.
Integer decoding validates this domain first, then returns `InexactNumber`
for a fractional value or `NumberRange` for a value outside the Rust type's
range; it never truncates or rounds. Native integer encoding validates this
domain, then writes the Rust type's plain base-10 `to_string()` without any
exponent, even for a representable `u128` with 39 integer digits.
These spellings keep inspected items and expressions readable and preserve
plain-digit equality with existing clients and emulator fixtures. Encoding
reports `EncodeFault::NumberRange` when needed.

TTL is enabled only by `#[tabulary(ttl)]` on `TtlSeconds` or its optional form.
At most one TTL attribute is allowed. Expired items still decode normally;
there is no hidden filtering or deletion. The caller supplies time values and
conditions when expiry affects behavior. Timestamp `new(i64)` and TTL
`new(u64)` constructors accept explicit epoch counts without reading time.
Timestamp helpers offer checked
`from_system_time`, `to_system_time`, and explicit unit conversions; overflow
and precision loss return errors.
`TimeError::{Overflow, SubunitPrecision, BeforeEpoch}` identifies conversion
faults; BeforeEpoch applies to unsigned TTL conversion, not signed timestamps.
All four wrappers expose `get()` for their declared epoch count. Each timestamp
offers `to_seconds`, `to_millis`, and `to_micros`, returning the named wrapper
or `TimeError`. Conversions reject any remainder before checking count overflow;
negative values are never rounded. TTL also offers `from_system_time` and
`to_system_time`; a pre-epoch input returns `BeforeEpoch` before precision checks.
SystemTime conversion checks the platform's representable range. Constructors
accept the full underlying integer range; the native integer codecs own wire
validation. None of these helpers reads the clock.

### 4.1 Custom converters and explicit defaults

```rust,ignore
#[tabulary(with = tabulary::convert::u128_string)]
external_number: u128,

#[tabulary(default = initial_quantity)]
quantity: u64,
```

A converter module defines a `Codec` implementing
`Converter<T> { type Wire: WireType; encode(&T, &mut EncodeContext);
decode(&AttributeValue, &mut DecodeContext); }`, with the same result types as
`Value`. Generated handles carry that codec. All expression constants use it
too, preventing an update from writing a different representation than a put.
Converters return structured `Custom { code: &'static str }` faults and cannot
override the owning path. No error value includes the rejected input.

`Record` and `Document` field declarations select `with = module` for the logical
type (the inner type for `Option<T>`). Projections inherit that selection.
Physical primary and index keys may use converters with an S, N, or B wire type;
key validation still checks the encoded value. A template source's field converter
controls its separately stored attribute; the template's component codec controls
the synthesized key and compares inverse-parsed logical values on decode.
Version and annotated TTL fields require their native codecs so their dedicated
operations retain the declared numeric representation. A flattened variants group
cannot have `with`: its discriminator stays native and its payload fields select
their own converters. A nested variants map may use a field converter.

`Native<T>` adapts `Value` to `Converter<T>`. The `field` helpers
`encode_optional`, `decode_optional`, and `decode_required` accept a context
already positioned at the physical field and use its selected converter.
They omit absent optional values and report `Missing` for absent required
values, including `Nullable<T>`. `Nullable::{Null, Value(T)}` implements `Value`;
`NULL(false)` returns `Custom { code: "invalid_null" }` because the fault
taxonomy has no dedicated malformed-null case. Its `NullableWire<W>` marker
uses `DYNAMO_TYPE = Unknown` for the union of NULL and W and does not inherit
scalar expression capabilities. This metadata does not permit unknown SDK
values; those still return `UnsupportedType`. `Option<Nullable<T>>` permits
absence, explicit null, and a value as three distinct states.
Encoding `Nullable::Value` rejects an inner codec's NULL output with
`Custom { code: "null_inner_value" }` rather than silently collapsing it into
the outer `Nullable::Null`. This includes nested nullable values.

A field's `default = path` names a visible `fn() -> T`, where `T` is the
declared field type (the whole `Option<U>` for an optional field). It runs only for a
missing field, never wrong types, NULL, malformed values, or failed validation.
Key fields, template sources, discriminator tags, and version fields cannot
have defaults. `Option` absence is its explicit presence rule, not a fallback
for corruption. Required lists, maps, sets, integers, and booleans have no
implicit empty/zero/false defaults.

`#[tabulary(validate = validate_order)]` adds a pure
`fn(&Order) -> Result<(), ValidationFault>` after construction and before
encoding. A validation fault uses an attribute handle, structured code, and
optional additional handle paths. It never calls a service or reads a clock.

### 4.2 Secrets

`Secret<T>` has `new(T)`, `expose_secret(&self) -> &T`, and
`into_inner(self) -> T`. Its `Debug::fmt` returns `Ok(())` without writing
anything, including a placeholder. It has no `Display` or `Deref`.
This is redaction, not encryption or guaranteed memory zeroization.

Errors, plans, and rendered expressions have hand-written Debug implementations
that omit all values. `request.input()`, `rendered.values()`, and
`error.sdk_error()` are explicit access to raw SDK objects and may expose
values. Redaction cannot survive a caller formatting a raw SDK input.

## 5. Status-shaped rows and unit enums

`#[derive(StringEnum)] #[tabulary(rename_all = "snake_case")]` on a unit enum
generates string values and strict decode. Per-variant `rename` overrides are
allowed; aliases and fallback variants are not implicit.
Every variant must declare `rename` or inherit `rename_all`, using the four
rules in §2; explicit names win. Empty or duplicate wire values are compile
errors, as are tuple and named-field variants (including empty payloads).
The derive implements `Value<Wire = wire::String>` and retains the caller's
record, document, and attribute path on decode faults. Non-string values use
the native string codec's `WrongType`/`UnsupportedType` faults. A
`#[tabulary(crate = "::renamed_dependency")]` override selects the runtime path.

Payload variants use a separate derive:

```rust,ignore
#[derive(tabulary::Variants)]
#[tabulary(tag = "deliveryStage", rename_all = "snake_case")]
enum DeliveryStage {
    Waiting { queued_on: tabulary::TimestampMicros },
    Sent { tracking_ref: String },
    Failed { failure_code: String },
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    delivery_ref: String,
    #[tabulary(flatten)]
    stage: DeliveryStage,
}
```

The discriminator is a string attribute at the row's top level. A field naming
rule on `Variants` applies to both payload fields and variant tag values.
Every variant must be a unit or named-field variant; tuple payloads are rejected.
At most one flattened variants field is allowed per record in the first version.
It cannot supply primary/index keys, TTL metadata, or a version attribute.
The enum can also be stored as one nested `M` without flattening.
Keep those physical schema inputs in common fields so the table schema remains
independent of the selected payload. Status-dependent sparse membership uses
explicit common optional index fields plus the record's validation hook; the
first version does not infer index inputs from a variant's payload.

The decoder reads the tag, selects one variant, then reads exactly its required
and optional attributes. It rejects payload attributes exclusive to another
variant even when unknown fields are otherwise allowed. Shared wire names across
variants are permitted only with identical Rust types, converters, and presence
rules. Unknown tag, wrong tag type, and missing tag are separate faults.

Shared-field compatibility uses the same written Rust type, converter path, and
explicit default path in each variant. The derive does not resolve type aliases
or prove that two converter paths name the same implementation. An omitted
converter means `Native<T>`. For a flattened group, the containing record or
document owns the unknown-attribute policy; for a nested `M`, the enum owns it.
Neither policy permits inactive payload attributes. Flattening adds no map
level to the depth count or field path. Nested storage adds one map level.

`DeliveryStageTags::WAITING` and peers are typed tags.
`DeliveryAttrs::STAGE.is(DeliveryStageTags::WAITING)` builds a discriminator
condition. Variant payload handles are scoped through
`DeliveryAttrs::STAGE.variant(DeliveryStageTags::SENT)`; their conditions
include the tag guard. They cannot be used as unconditional update targets.

Scoped payload access is available for a record's flattened group only. Nested
variant maps retain whole-map handles; their existing nested `Path` API supports
updates, not condition operands, so nested scoped predicates are deferred.
A generated singleton type for each tag selects a peer such as
`DeliveryStageSentAttrs<Delivery>`. Its named fields (for example,
`.tracking_ref`) are read-only `ScopedAttr` handles with the declared logical
type, field converter, and physical wire name. Shared fields retain the selected
tag's guard. Unit variants return an empty payload peer. Generic enums and
`generated_prefix` apply to these peers as to the other generated names.
The singleton tags convert into the group's ordinary `DeliveryStageTag` enum;
`.is(..)` and `transition(..)` accept either form. Use that enum for collections
of different tags.

Scope guards are requirements on the complete condition: composing predicates
with AND, OR, or NOT composes their payload expressions while retaining every
scope guard as an outer AND requirement. In particular, `NOT(scoped)` means
`NOT(payload) AND tag = selected`, and `scoped OR common` still requires the
selected tag. Combining scopes of different tags in one condition requires both
tags and therefore matches no item; use discriminator `.is(..)` conditions when
expressing alternatives across variants. Guards are retained in encounter
order without deduplication. Scoped handles expose no update target, update
operand, or conversion to an unguarded attribute; replace payloads with
`transition`.

`update.transition(DeliveryAttrs::STAGE, new_stage, expected_tag)` adds a
condition for the expected tag, SETs the new tag and payload, and REMOVEs the
union of old-only payload attributes. It replaces the entire variant value, so
stale fields cannot survive a status change. Common-field updates can join the
same request. Conditional old-item decode uses the old discriminator.

Milestone 31 preserves the discriminator `Attr` type. `Record` derives implement
`FlattenedVariants<V>` with the discriminator's field position, binding transition
values to the record's flattened enum rather than to nested maps or unrelated
tag attributes. This trait supplies generated metadata; manual typed transition
bindings are outside the supported API. Build validates the descriptor, key
protection, and disjoint common/variant namespaces.
Transitions encode the new enum at update build, then visit `Variants::ATTRIBUTES`
in its metadata order (generated tag first, followed by sorted payload names).
Present attributes become ordinary SET actions; every omitted group attribute
becomes REMOVE, including an absent optional field shared by two shapes. This
also defines same-tag replacement and transitions to unit variants.
The expected-tag condition is retained in `BuiltUpdate::condition()` and in
both rendering methods. Caller conditions are ANDed with that guard using the
same alias allocator; extracting `into_actions()` yields only the inspectable
mutation tree and discards its condition. No separate UpdateItem adapter exists
for transitions.

## 6. Strict decoding and error paths

The default unknown attribute policy is **deny** on full records and nested
documents. A visible `#[tabulary(unknown = "allow")]` permits unknown fields
without retaining them. A full put then drops unknown data: callers that need
preservation should use typed updates or model that data explicitly.
There is no automatic read-modify-write of unknown attributes.
Known attributes are always validated, including optional ones.

Decode order is deterministic: primary keys and their components, common fields
in declaration order, discriminator and active variant payload, derived indexes,
validation, then unknown attributes sorted by name. Inactive variant attributes
are forbidden regardless of the unknown policy.

```rust,ignore
pub struct DecodeError {
    pub record: &'static str,       // outer record's Rust name
    pub document: &'static str,     // current nested type's Rust name
    pub path: AttributePath,        // physical names plus indexes/components
    pub kind: DecodeFault,
}

#[non_exhaustive]
pub enum DecodeFault {
    Missing,
    WrongType { expected: DynamoType, actual: DynamoType },
    InvalidNumber,
    NumberRange,
    InexactNumber,
    InvalidLength { expected: usize, actual: usize },
    InvalidUtf8,
    InvalidSet(SetFault),
    UnknownVariant,
    InactiveVariantAttribute,
    UnknownAttribute,
    KeyFormat(KeyFormatFault),
    KeyMismatch,
    IndexMismatch,
    Validation { code: &'static str },
    Custom { code: &'static str },
    DepthLimit,
    UnsupportedType,
}
```

`AttributePath` is a list of `Attribute(String)`, `Index(usize)`, and
`Component(&'static str)` segments, not a preformatted dot string.
`error.attribute()` returns the first physical attribute.
Display escapes attribute names, so a literal dot is not confused with nesting.
An error in a line's nested price can name `Order`, the nested document,
and the precise list index. A missing whole document points at that field,
not an invented child.

`EncodeError` has the same record/path context and an `EncodeFault` covering
number range, nonfinite/negative-zero float, invalid/empty set, key format,
key size, custom validation, depth, and overflow. `BuildError` covers operation
constraints, contradictory/overlapping updates, invalid cursors, incompatible
indexes, version policy, empty operations, and service request limits.
All implement `std::error::Error`; all public fault enums are non-exhaustive.
Neither Display nor Debug includes attribute values or converter error text.

Depth is bounded at the DynamoDB document nesting limit. Checked size arithmetic
returns `SizeOverflow`. Unsupported future SDK `AttributeValue` variants
produce a structured `UnsupportedType` fault, never a panic or a default.

Partial projections have their own decoders (§9). They never call
`Record::from_item` and never fabricate a full record with missing fields.

## 7. Expression system

Expressions use owned AST nodes with a record owner parameter. No caller writes
attribute names or placeholder tokens. Handle codecs encode constants at build.
`Condition<R>` and `Update<R>` cannot contain another record's handles.

| API on a handle/path | Availability and meaning |
| --- | --- |
| `exists()`, `not_exists()` | Every path; render attribute_exists / attribute_not_exists. |
| `eq(T)`, `ne(T)`, `eq_path(other)` | Same logical type and codec; explicit value or path comparison. |
| `lt(T)`, `le(T)`, `gt(T)`, `ge(T)`, `between(lo, hi)` | Ordered numeric, string, or binary wire categories only. |
| `begins_with(T)` | String or binary, including key sort predicates when valid. |
| `contains(element)`, `size()`, `in_values(values)` | Typed collection/string capabilities; IN length checked against the service limit. |
| `set(T)` | Mutable required/optional field; optional None is not an operand. |
| `remove()` | Optional fields; indexed list elements through an explicit remove-element action. Required fields cannot be removed. |
| `add(delta)`, `delete_members(set)` | Top-level native numbers/sets for ADD; sets for DELETE. |
| `if_not_exists(T)` | Typed operand usable on the right side of SET. |
| `increment(delta)` | Numeric SET with the existing path plus delta; missing values fail unless if_not_exists is explicit. |
| `decrement(amount)` | Numeric SET with path minus amount, allowing unsigned fields to subtract a positive typed value. |
| `list_append(List<T>)`, `append_or_empty(List<T>)` | SET list_append(path, value); the latter explicitly uses if_not_exists(path, empty-list). |
| `field(DocumentAttrs::FIELD)` | Compose a known nested document path; root owner is preserved. |
| `at(index)` | List element path, checked nonnegative index. |
| `entry(MapKey::new(data_key)?)` | Dynamic map data keys are aliased as one segment, never parsed as schema names. |

`.set_expr(path.if_not_exists(value))` and
`.set_expr(path.if_not_exists(value).plus(delta))` provide explicit SET
operands. Optional numeric fields can be initialized this way; the default is
written at the mutation. `Update::new().and(action).and(action)` composes
actions. Conditions use `.and`, `.or`, and `.not`, with explicit grouping.
SET operands also support `.minus(amount)`. Operand typing cannot prove that
server arithmetic stays in a Rust field's range. Callers add range guards next
to mutations when underflow or overflow would violate the model.
Top-level number ADD initializes an absent DynamoDB number by service semantics;
use SET plus a condition when that behavior is unwanted.

Primary keys and their source components have no mutation methods. Version,
derived index, discriminator, and variant fields have the dedicated APIs
described here. A custom converter must declare its wire capabilities; it
cannot gain numeric operators from a logical `u128` stored as a string.
Cross-codec comparisons are not inferred.

The condition API declares these capabilities through public `condition::Ordered`,
`Prefix`, and `SizedWire` traits on wire markers. `condition::Contains<T>` is a
codec capability with an associated logical `Element` and `ElementCodec`.
Native string, list, and set codecs provide it; custom codecs opt in explicitly.
The equality-only `u128_string::Wire` implements none of these capabilities.
`size()` produces a `u64` operand. `in_values(Vec<T>)` accepts 1 through 100
values; empty input returns `EmptyOperation`, and excess input returns
`RequestLimit` before encoding. Constants are owned (`'static`) and encoded by
`Condition::build`, which returns a record-owned `BuiltCondition` with an
inspectable tree and redacted Debug. This build step validates descriptors and
values; textual rendering and expression-size validation belong to later stages.

Update actions similarly encode only at `Update::build`, returning a record-owned
`BuiltUpdate` with inspectable actions and redacted Debug. Milestone 22 adds
`Path` handles composed with `field`, `at`, and `entry`. Document derives publish
`Document::FIELDS` for descriptor validation; manual document codecs may provide
physical metadata, but typed field handles still come from derives. The default
empty slice refuses traversal.
Traversal uses native document/list/map codecs; leaf handles retain their codec.
List indexes accept integers convertible to `usize`; negative or overflowing
indexes fail at build with `invalid_list_index`. Indexed elements have the
explicit `remove_element()` action; dynamic entries are optional and use
`remove()`. `MapKey::new` rejects an empty key and preserves all other characters
as one literal segment. Protected access is retained through every ancestor.
`operand()` reads a typed path on the SET right
side, including protected fields, while only mutable handles provide targets.
`SetOperand` supports one `.plus(T)` or `.minus(T)`, returning a `SetExpression`
that cannot chain arithmetic, matching DynamoDB's SET grammar. Both preserve the
owner, logical type, and codec of the source; `set_expr` requires these to match
the target. `update::Numeric` is the opt-in wire capability for SET arithmetic.
ADD and DELETE remain restricted to `Native<T>` codecs with `update::Addable`
and `SetWire` wire categories. `update::Append<T>` is an explicit list-codec
capability that supplies the logical empty value for `append_or_empty`; native
lists implement it. Ordinary `list_append` never inserts a missing-list default.

Nested updates do not create missing parent maps. DynamoDB can reject such a
path; the caller must create the map explicitly or choose a put.
The builder rejects duplicate and ancestor/descendant target paths, including
targets contributed by version, transition, and reindex expansion. A pair of
different nonoverlapping list indexes is allowed; service list-index semantics
apply. No reordering into an imagined sequential execution takes place.

### 7.1 Rendering and review

`BuiltCondition::render()` returns a standalone `RenderedExpressions` with fresh
aliases and only `condition()` populated. It traverses the validated tree
left-to-right, encloses AND/OR groups in parentheses, and writes NOT as
`NOT (child)`. Rendering does not re-encode constants. Request-wide size and
substitution checks run in operation builders after all fragments are rendered.

`BuiltUpdate::render()` returns standalone update text and bindings.
`render_with_condition(&BuiltCondition<R>)` allocates the update first and then
the condition in the same alias space, matching the example below. Overlap
validation runs at update build on physical segment vectors, across all groups,
and returns `OverlappingUpdate` with both paths. Declaration order in the built
actions remains inspectable; only rendering groups actions by their keyword.

`plan.expressions()` returns `RenderedExpressions`, with
`condition()`, `update()`, `key_condition()`, `filter()`,
`projection()`, `names()`, and `values()`. The first five are optional
text. The maps contain the SDK attribute names and values actually sent.
`plan.input()` returns the full SDK input by shared reference.

The renderer aliases every attribute segment, including ordinary nonreserved
names. Placeholder assignment is deterministic AST traversal order:
`#n0`, `#n1`, ... and `:v0`, `:v1`, ... . Names are deduplicated by
segment; values are not deduplicated. All expressions for one request share one
allocator; every transaction action gets its own allocator as the API requires.
There are no unused bindings. Update action groups render in SET, REMOVE, ADD,
DELETE order, preserving declaration order within each group.

For `quantity.set(7)` guarded by `quantity.eq(6)`, the complete output is:

```text
UpdateExpression:    SET #n0 = :v0
ConditionExpression: #n0 = :v1
Names:                #n0 -> quantity
Values:               :v0 -> N("7"), :v1 -> N("6")
```

The compiler reports known invalid type combinations. Build validates value
limits, expression text byte lengths, substitution limits, path depth, and
operation-specific grammar. Free-form expression strings are deliberately
excluded from the typed API. Users needing an unsupported AWS feature can use
the AWS SDK directly and its raw `AttributeValue` types.

Milestone 46 adds `RenderedExpressions::validate()`, called by every operation
builder after rendering its complete expression set, including automatic guards
and BatchGet transport-key projections. Each expression has a 4 KiB UTF-8 text
cap and a 300-operator/function cap. Comparisons, logical operators, BETWEEN,
IN, and functions count; SET assignment and the AND separator inside BETWEEN
do not. Each path permits 32 dereferences after its root, on both update targets
and operands. Literal punctuation in aliased names consumes no path depth.
Placeholder identifiers have a 255-byte cap; this does not cap bound contents
at 255 bytes. All shared names and values together have a conservative 2 MiB
JSON bound, counting escaping, base64, tags and map punctuation through the same
wire estimator used by BatchWrite. This choice may reject bindings near the
service boundary; it avoids treating approximate billed number bytes as wire
bytes. Transaction actions validate their own maps independently.
These limits follow the [AWS expression constraints](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/Constraints.html#limits-expression-parameters)
and [document path rules](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/Expressions.Attributes.html#Expressions.Attributes.DocumentPaths).

## 8. Prepared operations and SDK integration

`Table::<R>::new(table_name)` validates a nonempty table name or ARN and returns
a binding. It does not describe or create the table. Record definitions never
embed deployment table names.

Milestone 25 keeps the merged metadata-only `Record` trait: `Table<R>::get`
requires `R: Record + Document` and accepts an owned `K: RecordKey<R>`.
Full reads use `Document::decode_map`; selected reads use the projection's own
decoder. Neither requires a change to generated record or key methods.
`Table::new` rejects an empty identifier and preserves other names or ARNs
verbatim; service-side identifier syntax and existence remain caller concerns.

Each operation builder has `build() -> Result<Prepared..., BuildError>`.
Encoding failures are retained as `BuildError::Encode(EncodeError)`.
The resulting plan owns the SDK input and its decode metadata. It is immutable
and can outlive all records used to create it.

| Table method | Prepared input | Decoded success |
| --- | --- | --- |
| `get(key, consistency)` | `GetItemInput` | `GetResult<P> { item: Option<P>, capacity }` |
| `put(&record)` | `PutItemInput` | `WriteResult<R> { item: Option<R>, capacity, item_collection_metrics }` |
| `delete(key)` | `DeleteItemInput` | Same, optionally ALL_OLD. |
| `update(key)` with `changes(Update<R>)` | `UpdateItemInput` | Same, optionally ALL_NEW or ALL_OLD. |
| `query(key_condition, consistency)` | `QueryInput` | One `Page<P, Cursor<R, I>>`, where `I` is the concrete primary peer (for example, `OrderPrimary`). |
| `scan(consistency)` | `ScanInput` | One page, optionally bound to a parallel segment. |
| `batch_get(keys, consistency, budget)` | Explicit batch state (§10) | Records/missing keys in input order, or typed partial error. |
| `batch_write(actions, budget)` | Explicit batch state (§10) | Processed action identities and pending actions. |
| `check(key, condition)` | Transaction-only action | One typed condition check; never a standalone network call. |

`P = R` until a projection is selected. Get, query, scan, and batch get all
require a consistency argument, with `ReadConsistency::{Strong, Eventual}`.
There is no default method or default implementation for this choice.
Index query and index scan follow the same contract (§3.2).

Every prepared type has:

```rust,ignore
fn input(&self) -> &aws_sdk_dynamodb::operation::get_item::GetItemInput;
fn expressions(&self) -> &RenderedExpressions;
fn apply(&self, builder: GetItemFluentBuilder) -> GetItemFluentBuilder;
fn decode(&self, result: Result<GetItemOutput, SdkError<GetItemError>>)
    -> Result<GetResult<P>, ReadError<GetItemError>>;
```

Other operations use their corresponding SDK types. This is a family of
monomorphic adapters, not one untyped map of operation names.
`apply` copies every request field, including absent fields, into the supplied
SDK fluent builder. It leaves the client's transport configuration alone.
Calling it performs no I/O. Callers may configure SDK interceptors and transport
settings normally; changing request fields afterward invalidates the typed
plan's decode assumptions.

Use `let raw = request.apply(client.get_item()).send().await;` then
`let output = request.decode(raw)?;`. The application may instead inspect
`input()` and use the SDK itself; it remains responsible for sending that exact
input and passing the matching result to the decoder.

`.capacity(Capacity::None | Total | Indexes)` controls ReturnConsumedCapacity;
default `None` means not requested. Preserve the SDK's complete
`ConsumedCapacity`, including table/index details, without inventing zeroes.
Transactions and batch rounds preserve the full vector for each request.
Decode errors preserve capacity and pagination metadata received with the
response, so a malformed item does not erase accounting information.

For milestone 25, `Capacity::None` (including the default) omits the SDK field;
the other choices send `TOTAL` or `INDEXES`. `PreparedGet<P>` aliases the shared
immutable `PreparedRequest<GetItemInput, P>` storage. Its `input()` and
`expressions()` describe exactly what `apply` copies, including absent legacy
fields. `ReadError<E>` holds either a boxed typed `SdkError<E>` or a boxed
`ReadDecodeError` with `position`, structured `source`, and unchanged `capacity`.
GetItem's fault position is zero; `raw_items()` explicitly exposes its one raw
item. `sdk_error()` exposes the original SDK error. Error and prepared-request
formatting redacts payloads and SDK text. Pagination metadata joins these read
errors with the page adapters; GetItem has no pagination metadata.

`.return_item(ReturnOld::None | AllOld)` selects success returns for put/delete;
update uses `.return_item(ReturnItem::None | AllOld | AllNew)`.
The separate enums make unsupported operation choices unrepresentable.
UPDATED_OLD/UPDATED_NEW are excluded in the first API because they are partial
shapes and cannot decode into a full record. A later API can add typed update
projections. No read follows a write to obtain return values.

The shared `write_options` types supply these choices to milestones 27–29.
Both return enums default to `None` and convert to the SDK's `ReturnValue`;
`ReturnOld` also converts to `ReturnItem`. The conversions send `NONE`,
`ALL_OLD`, or `ALL_NEW` explicitly. Failure-side `OldItemMode` is independent.
`.collection_metrics(CollectionMetrics::None | Size)` selects the SDK's
`ReturnItemCollectionMetrics` (`NONE` or `SIZE`), defaulting to `None`.
`WriteResult<R>` retains the optional full record, complete consumed capacity,
and item-collection metrics supplied by the SDK, without inventing accounting.

Milestone 28 adds `Table<R>::delete(key)` for `R: Record + Document` and
`K: RecordKey<R>`. `DeleteBuilder<R, K>` owns the table name and key, encoding
the key and validating its optional `Condition<R>` at `build()`.
Repeated `.condition(...)` calls AND their predicates, matching UpdateItem.
Query and Scan follow the same rule for repeated `.filter(...)` calls:
every supplied predicate is ANDed with the preceding filters.
Callers can also combine predicates with `.and(...)` or `.or(...)`.
`PreparedDelete<R>` aliases
`PreparedRequest<DeleteItemInput, R>`, copies every SDK input field, and derives
failure-side old-item mode from its immutable input. Success attributes are
strictly decoded as the complete record: absent attributes mean `None`, while a
present empty or malformed map yields `WriteError::Decode` with unchanged raw
attributes, capacity, and collection metrics. Refusals use the milestone 26
classifier without changing their kind when an old item is absent or invalid.

Milestone 29 adds `Table<R>::update(key)` for `R: Record + Document` and
`K: RecordKey<R>`. `changes(Update<R>)` replaces the complete mutation set;
missing or empty changes fail at build. Repeated `condition(Condition<R>)`
calls AND their predicates. Build uses milestone 22's validated update and
shared condition renderer, then owns the complete `UpdateItemInput` in
`PreparedUpdate<R>`, an alias for `PreparedRequest<UpdateItemInput, R>`.
`apply` replaces every modeled input field, including absent legacy options.
SDK failures use milestone 26's classifier with the requested `OldItemMode`.

Update success returns must agree with the selected mode. `AllNew` requires
attributes; `AllOld` permits absence when the update created an item; `None`
rejects any present attributes, including an empty map. Every present full
return is decoded strictly. Mode/shape mismatches are committed-write
`WriteError::Decode` faults with `DecodeFault::Validation` codes
`missing_returned_attributes` or `unexpected_returned_attributes` at the record
root. `WriteDecodeError` preserves capacity and collection metrics unchanged;
its raw map is empty when no attributes were supplied. No follow-up read occurs.

An absent GetItem item is `None`. A nonempty or explicitly present item that
fails decoding is an error, never “not found.” An SDK response with an empty item
map decodes as an item and fails on required keys; only an absent item is missing.
Query and scan decode each returned item strictly. A corrupt page yields
`ReadError::Decode` with item position, source, received metadata, and raw
payload behind explicit access. No row is skipped and no cursor is advanced
implicitly.

The builder validates known limits before sending, including primary key byte
lengths, item size bounds, batch/action counts, and expression grammar.
Constraints that require existing item data remain service checks.

## 9. Query, scan, projections, and opaque cursors

### 9.1 Key conditions and filtering

`Order::primary().partition(order_ref)` returns a typed key condition.
It requires partition equality. When a sort key exists, it offers exactly one
of `sort_eq`, `sort_lt`, `sort_le`, `sort_gt`, `sort_ge`,
`sort_between`, or `sort_begins_with`. The last requires string/binary sort
keys. Hash-only schemas expose no sort operators. The type state prevents
omitting partition equality, adding a second sort predicate, OR, or using a
non-key attribute. Each index exposes its own distinct key-condition type.

`KeySchema<R>` is the common query-facing contract with logical `Partition`,
`PartitionCodec`, `Sort`, and physical `KEYS`; primary peers additionally implement
`PrimarySchema<R>`, which inherits this contract. Index peers retain their
`IndexSchema<R>` metadata and implement `KeySchema<R>` using the same types and
physical positions. The metadata-only `Record` trait remains unchanged.
`KeyCondition<R, I>` defaults its sort state to the schema's `Sort`; every sort
operator consumes that slot and returns the `SortComplete` state. `build()`
returns `BuiltKeyCondition<R, I>` with inspectable `root()` and `render()`;
rendering populates only `key_condition()` through the ordinary condition
renderer and request-local aliases. Both unbuilt and built Debug omit values.
Key constants validate the physical descriptor, codec wire type, scalar domain,
and the strictest byte limit across all primary/index uses of that physical
attribute before rendering. Native string/binary codecs
implement `PrefixCodec<T>`; custom codecs must opt in and also declare the
`condition::Prefix` wire capability. Composite codecs implement it with their
generated complete-boundary prefix carrier as the associated `Prefix` input.

Ordered sort predicates additionally require `OrderedKeyCodec<T>`, an explicit
promise that encoded order preserves logical input order. Native field-keyed
sorts retain their existing wire capabilities; custom codecs opt in separately.
For composite templates, the derive supplies this capability only when every
component is fixed width and order-preserving (`dec20` or fixed-width `hex`),
except that the last component may be `text` if it ends the template. Literal
prefixes and separators between fixed-width components do not change order.
Unknown widths are conservatively excluded. Literal-only templates preserve
the order of their sole `()` input.

`dec`, signed decimal, variable-width `hex`, and nonfinal `text` do not receive
this capability. Final `text` with a trailing literal is also excluded:
`{label:text}!end` reverses the order of `"a"` and `"a "`. The five ordered
operators therefore fail to compile for these templates, with a diagnostic
pointing to the first offending component and suggesting `dec20` or
`sort_begins_with`. Equality and complete-boundary prefixes remain available.

Query builders accept `filter(Condition<R>)`, `limit(NonZeroU32)`,
`order(QueryOrder::Ascending | Descending)`, `select::<P>()`, and
`resume(Cursor<R, I>)`. Key fields are rejected in query filters; express
them in the key condition. Scan filters may include key fields.
For an index query this restriction refers to the selected index's physical keys;
base-table keys that are not keys of that index may be filtered. Non-ALL index
reads also reject filters using attributes outside the stored index namespace,
so filtering cannot introduce an implicit base-table fetch.
Filters act after service evaluation and do not reduce the read work. A limit
is an evaluated-item limit per call, not a requested count of matching records.
Query defaults to ascending traversal and an omitted limit. `NonZeroU32` limits
above the SDK's `i32::MAX` are build errors rather than clamped values. Decode
preserves the service's row order and never sorts, truncates, or fills a page.

`Page` contains `items`, `next`, `count`, `scanned_count`, and
`capacity`. Empty items with a nonempty last-evaluated key still produce
`Some(cursor)`. Only an absent or empty last-evaluated-key map means end.
There is no automatic paginator or collection loop. Pages and scans are not
snapshot isolation, even when each read is strong.

Milestone 34's shared `page` module retains the SDK's `i32` counts. It refuses
negative counts, a Count that differs from the returned item count, a ScannedCount
below Count, and invalid service continuation keys. These page-level faults use
position zero and a structured custom fault at `Count`, `ScannedCount`, or
`LastEvaluatedKey`; item faults retain their zero-based response position.
`ReadDecodeError::page` is `Some(PageMetadata)` for page reads and `None` for
GetItem. It preserves the original count, scanned count, and absent/empty/nonempty
continuation map even on failure; `last_evaluated_key()` explicitly exposes the
raw map. The entire raw page remains available through `raw_items()`; no partial
success page or resumable cursor is returned on a corrupt page.

### 9.2 Typed projections

```rust,ignore
#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct OrderQuantity {
    #[tabulary(attribute = OrderAttrs::QUANTITY)]
    quantity: u64,
}
// orders.get(key, Strong).select::<OrderQuantity>().build()?
```

The projection refers to handles, not strings. Derive checks owner, logical type,
presence, and codec. A required source cannot silently become optional.
An optional projected source remains `Option<T>`. Converters are inherited
from the record, not redeclared. A projection is read-only: it has no Record
implementation, put, key inference, or update method.

Only selected attributes are required on projection decode. Extra attributes
explicitly fetched as transport metadata are allowed and validated separately;
the base record's unknown policy does not reject them. Requested known attributes
remain strict and inherit any explicitly declared source-field default.
Whole-record validation hooks do not run on an incomplete view. Nested projections
use another `Projection` with the nested
document as source. Selecting a flattened variant requires the whole tag/payload
group; disjoint variant fields without a discriminator are rejected.

The first Projection derive accepts concrete, owned named-field structs. Its
`record = Source` may name a concrete instantiation of a generic source. Mark a
nested view field `#[tabulary(attribute = SourceAttrs::FIELD, nested)]`; its
type is another Projection (or `Option<View>` for an optional source). Nested
selection fetches the whole physical map but decodes only the nested view.
It requires the native document-map codec; a custom converter cannot be bypassed.
An inherited nested default is projected by moving its selected fields, without
encoding the source or applying child defaults again. Existing `None` values
remain `None`; unselected fields cannot reject the view.
Select a flattened variant through its existing discriminator handle, with the
whole Variants enum as the view field type; selecting just the tag is invalid.
The projection's `attributes()` lists the complete physical selection.
`check_available(available, transport)` checks non-ALL compatibility against the
index's full stored namespace (keys plus INCLUDE fields), including separately
requested transport keys. Index request builders supply that namespace once
index metadata is implemented; this check never fetches missing attributes.

Batch get automatically adds complete primary keys for response correlation,
even when the view omits them, and strips them from the user's view.
`RecordKey::decode_key` decodes only physical keys, parsing composite components
without requiring separately stored sources; full-record decode still requires
and compares those sources. Batch identity compares canonical physical key
values, including normalized numeric values, rather than Rust field equality.
Query/scan use the service's last-evaluated key directly; they never infer
pagination from the last projected item. A non-ALL index must contain all
requested projection attributes, including any transport keys.

### 9.3 Cursor binding and serialization

`Cursor<R, I>` has private fields and a Debug implementation that omits keys.
It contains the complete service last-evaluated-key map and a binding to the
table, index, schema signature, operation, key condition, filter, projection,
consistency, direction, and scan segment. It validates scalar key types but
does not decode the map as a record. Index cursors retain both base and index
keys exactly as received.

Milestone 33 implements the in-memory binding. Milestone 36 is split at its byte
boundary: 36a adds encoding and size checks; 36b adds the untrusted-byte parser.
The schema signature compares the
complete physical `RecordSchema` metadata (name, field descriptors, primary keys,
and index definitions), by value without hashing or Rust type identity. Codec
implementation bodies and nested document decode rules are not represented by
that metadata. Projection identity is the sorted, deduplicated physical attribute
set returned by `Projection::attributes()`, with a full-record read distinct from
an explicit selection. Two view types fetching the same physical attributes have
the same request binding, including nested views that fetch a whole map.
Condition identity retains AST operators, grouping, operand order, and wire tags;
numeric constants normalize through `Decimal`, maps sort by name, and sets sort
by normalized members. It does not rewrite logically equivalent boolean trees.
Page limits and capacity reporting are excluded. Internal page-adapter hooks
construct a cursor only from a nonempty service map, validate scalar keys and the
required base/index key attributes, and preserve the original map unchanged.

`resume` checks this binding, so a cursor for a different query, table, schema,
index, or segment is a `CursorMismatch` build error. Per-page limit is not part
of the binding and may change. Query and scan cursors cannot interchange.

`cursor.to_bytes()` and `Cursor::from_bytes(&bytes)` use a versioned,
length-delimited binary format. The encoded cursor cap is 4 MiB; depth is at most
the service document nesting limit. Check lengths before allocation and reject
an oversized binding at request build with `CursorTooLarge`.
The binding uses canonical schema/AST/typed-value bytes, not
`DefaultHasher`, Rust type addresses, or unstable Debug text. Map keys are
sorted. There is no schema hash collision dependency.
V1 rejects unknown format versions; it does not promise cross-major-version
resumption. Reject invalid keys and trailing bytes. Empty key maps cannot
construct a resumable cursor.

`to_bytes()` returns `Result<Vec<u8>, BuildError>`. V1 starts with the byte `1`,
followed by the canonical binding and the complete key map sorted by name.
Lengths and collection counts are big-endian `u32`; schema positions are
big-endian `u64`. Tagged schema, expression, and value fields retain the existing
structural identity, including declaration order in schema metadata. Key number
spellings remain unchanged; only binding constants normalize through `Decimal`.
The size check at request build counts the binding and the minimum envelope
without allocating an output buffer. Encoding also checks the complete cursor,
including service keys, against the 4 MiB cap. Expression trees, as well as nested
document constants, have a maximum depth of 32; excessive depth is `InvalidCursor`
at request build.

`Cursor::from_bytes(&bytes)` returns `Result<Cursor<R, I>, CursorError>`.
It checks the outer cap, lengths, counts, tags, UTF-8, numeric domains, depth,
canonical map/set ordering, and required scalar keys before copying keys and
binding data into owned storage. Cursor errors distinguish oversized, truncated,
trailing, unknown-version, invalid-format, invalid-key, and excessive-depth input
without retaining token contents. Parsed schema/request bytes stay private and
are compared with the request's complete canonical binding at `resume`, so
parsing under a different record marker does not erase a schema mismatch.

The key map contains exactly the union of base-table and selected-index key
attributes (at most four distinct names); additional attributes are refused.
Both parsing and in-memory construction enforce this rule. String/binary
partition values have a 2048-byte cap and sort values a 1024-byte cap. Numeric
key text has a 192-byte cap before Decimal parsing in both construction and
untrusted-byte parsing. This bounds work on untrusted text, rather than predicting
the endpoint's formatting. With 38 significant digits and adjusted exponents
from -130 to +125, the longest nonredundant negative positional fraction needs
170 bytes: one sign, `0.`, 129 leading fractional zeros, and 38 digits. The
largest negative integer needs 127 bytes (one sign plus 126 integer digits).
The cap adds 22 bytes of spelling margin to the 170-byte worst case and also
covers compact exponent notation, including uppercase `E` and explicit `+`.
Longer spellings, including redundant zeros, are refused even when they would
normalize into the numeric domain. Bounded original spellings are retained.
The parser extracts required key descriptors into four borrowed slots, then
validates every key exactly once before constructing owned data.

Opaque does not mean encrypted or authenticated. Tokens contain key and filter
values. Applications exposing them outside a trusted boundary must authenticate
and authorize them, or store them server-side. Matching a cursor is never access
control. The library does not fetch state, sign tokens, or assign tenancy.

### 9.4 Parallel scans

`ScanSegment::new(segment, total)` requires a positive total and
`segment < total`, within DynamoDB's supported segment range.
Milestone 35's segment value has private `u32` fields and rejects total zero,
totals above 1,000,000, and segment IDs at or above total with
`OperationConstraint { code: "invalid_scan_segment" }`. This also rejects
out-of-range values before converting them to SDK integers. The boundary is
defined by the [Scan API](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_Scan.html).
`table.scan(consistency).segment(segment).resume(cursor).build()` prepares one
page of one segment. An unsegmented scan has a distinct cursor binding.
The Scan builder also accepts `filter`, `limit`, `select`, and `capacity`, with
the same read rules as Query; Scan filters may include key attributes.
Table scans use `Cursor<R, ()>` because `Record` remains metadata-only.
Index scans use `Cursor<R, I>` with the generated
index peer. Both retain the full schema and operation binding described above.
Changing total or segment rejects the old cursor. Each segment keeps its own
cursor; the application schedules concurrent calls and bounds memory.
There is no automatic fan-out, merge order, or cross-segment progress storage.

## 10. Batches and bounded unprocessed work

Milestone 42 supplies the pure `BatchGetProgress<R, K, P = R>` core and
`BatchBudget`, using the same `R: Record + Document` and `K: RecordKey<R>`
contracts as GetItem. `new(keys, budget)` encodes and validates 1--100 owned
keys and rejects duplicate canonical physical keys without requiring Rust Eq.
`next_round()` reserves a call and returns a `BatchGetChunk` with immutable
encoded keys and original input positions. Issuing a chunk consumes a budget
slot even if the caller never sends it; this conservative reservation cannot
be refunded. A live chunk prevents another round, even on the final slot.
`accept_round(&chunk, found, pending)` receives already decoded values paired
with original input positions, plus unprocessed positions. It validates batch
identity, round, subset membership, duplicates and disjointness before mutation;
invalid submissions leave the round outstanding and progress unchanged.
`entries()` exposes ordered Found/Missing/Pending states; `into_partial()` moves
owned keys and records without Clone. Core `finish()` rejects outstanding or
pending work, distinguishing unused calls (`PendingWork`) from exhausted budget.
Its complete result contains ordered records; incomplete results retain ordered
entries and issued calls. SDK requests, consistency/projection selection,
capacity vectors, response-key correlation, and terminal SDK/protocol/decode
error wrapping and freezing belong to milestone 43's adapter below. The adapter
must consume the core into a partial result on terminal failure; the low-level
core's nonmutating rejection is not permission to retry a failed SDK call.

Milestone 43 stages the SDK adapter and projection selection in two PRs.
`Table::batch_get(keys, consistency, budget)` validates immediately and returns
`BatchGet<R, K>`. `next_request()` prepares an immutable `PreparedBatchGet<P>`
using the shared prepared-request storage; `input()`, `expressions()` and `apply`
expose/copy the complete SDK input. `decode` returns an opaque `BatchGetRound<P>`
containing its issued identity and either decoded values or a typed fault.
`accept` validates that identity before accounting or changing progress.
`BatchGetError<K, P> { kind, partial }` boxes its owned partial; the kind is
`BatchGetFailure::{State, Sdk, Decode, Aborted}`. Budget exhaustion is
`State(BatchStateError::BudgetExhausted)`, rather than a separate batch enum.
The complete and partial core result types now have `capacity` vectors; pure
core operations leave them empty. The SDK adapter preserves one complete vector
per accepted round, including an empty vector when the SDK supplies no accounting.
A malformed or unexpected physical key, wrong table, duplicate or overlapping
response is `State(InvalidResponse)`: none of that round's item states is trusted.
Once all keys correlate, decoding retains every valid record from that round;
failed positions remain Pending and `Decode { failures }` lists every malformed
item as its original input position paired with its structured decode error,
including the attribute path. The vector follows response order; it distinguishes
these corrupt items from the service's unprocessed keys in the partial, and
`source()` exposes its first error. Prior progress and all received capacities
survive both paths. A terminal error transfers ownership to its partial and
subsequent operations return Frozen (their partial is empty, already transferred).
`abort_round(Cancelled | Deadline)` freezes an outstanding round; `finish` returns
the abort cause and all owned progress. Aborting with no outstanding round is
WrongRound. An aborted round adds no capacity vector, since no result was accepted.
Before the first issued round, consuming `.select::<V>()?` changes the output
type using milestone 24's projection checks and the shared read-selection gate.
It rejects empty selections and configuration after issuing a round. Its SDK
projection lists the selected attributes followed by any missing physical primary
key attributes, deduplicating keys already selected. Correlation validates those
full physical keys even when the view omits them; the view then decodes only its
selected fields with the source codecs. Each later request retains that projection
and explicit consistency rather than trusting options echoed in UnprocessedKeys.
The unchanged `06_query_batch.rs` example is registered as a real compile test.

The design resolves two requirements together: unprocessed items cannot
disappear, and tabulary cannot retry or hide calls. Batch state prepares the
next request only when the caller asks. It performs no I/O and does not sleep.
All subsequent unprocessed submissions are visible calls in application code.

The first version has a homogeneous record type and one table per batch. The
application can run several batches for several tables. Batch get rejects more
than 100 keys, duplicates, and empty key lists; batch write rejects more than
25 actions, duplicates of the same table/key, and empty action lists. There is
no implicit chunking. Wire request-size checks are separate from item-size
checks. The service may return fewer results because of its response limits.
See [BatchGetItem](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_BatchGetItem.html)
and [BatchWriteItem](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_BatchWriteItem.html).

`BatchBudget::new(max_calls)` requires a positive integer and counts the initial
request. It is mandatory, not defaulted. For example, a budget of three permits
at most three explicitly issued batch requests. It bounds SDK send invocations,
not internal SDK attempts, elapsed time, or service latency.

```rust,ignore
let mut batch = orders.batch_get(keys, ReadConsistency::Strong, BatchBudget::new(3)?)?;
batch.set_capacity(Capacity::Total)?;
while let Some(request) = batch.next_request()? {
    let result = request.apply(client.batch_get_item()).send().await;
    batch.accept(request.decode(result))?;
    // The application may apply its own delay or deadline before the next call.
}
let complete = batch.finish()?; // Never succeeds while work is unprocessed.
```

`next_request` returns None when complete or budget-exhausted. It returns an
error if another request is outstanding. A round can be accepted only once,
for that batch identity and round. It contains only unprocessed keys/actions
from the preceding response. `accept` checks that they are a subset of the
issued request; unexpected responses are protocol faults. The bounded state is
per operation, not global configuration.
`set_capacity(Capacity)` is available on either batch state before the first
round; changing it after a round is issued returns `AlreadyStarted`.
`accept` takes a round Result, so it can attach accumulated progress to
SDK, protocol, or item-decode failures without requiring records to be Clone.
On such a failure it transfers the partial result into the returned BatchError
and freezes that state against further requests.

Batch get matches responses by their validated full primary keys, independent
of response order. A key omitted from both returned items and UnprocessedKeys
is `Missing`. Unprocessed keys remain `Pending`, never missing. Successful
`finish` returns `BatchGetComplete<P> { records: Vec<Option<P>>, capacity }`
in original input order. A malformed returned item causes a typed decode error
with input position and path; accumulated successes and unprocessed keys remain
available in the error's partial result.

Milestone 44 adds the table-independent `BatchWriteProgress<R>` core alongside
BatchGet. `BatchAction::put(&record)?` freezes the same encoded item and item-size
gate as prepared put; `BatchAction::delete(key)?` freezes the prepared-delete key
encoding. Target identity is extracted from these payloads without running a
second codec, validates both physical key fields, and normalizes numeric spelling.
`new(actions, budget)` requires 1..=25 actions and rejects duplicate targets.
The later adapter binds this homogeneous action list to one table.
`next_round()` reserves a call and returns an immutable `BatchWriteChunk` of
original positions. The corresponding actions remain available through
`entries()`. `accept_round(&chunk, pending)` validates identity, round and the
entire unprocessed subset before marking all other issued positions `Processed`.
Only `Pending` actions can appear in another round; budget and lifecycle errors
reuse `BatchStateError`. `finish()` returns ordered actions on completion, or
ordered action/state entries and issued-call count on incomplete work.
`fail_round(&chunk)` marks outstanding actions `Indeterminate` and freezes the
core: further rounds and completion return `BatchStateError::Indeterminate`.
Consuming an outstanding core through `into_partial()` or incomplete `finish()`
also conservatively marks its pending writes indeterminate. Already processed
actions retain their state. The adapter in milestone 45 retains SDK and abort
causes, validates raw unprocessed responses, and records capacity vectors; none
of those SDK/error-wrapper surfaces are introduced by this pure core.

Milestone 45's adapter uses `Table::batch_write(actions, budget)` to construct
`BatchWriteBatch<R>`. `next_request()` builds an owned `PreparedBatchWrite`;
`input`, empty `expressions`, and `apply` follow the single-item adapters.
`decode` consumes that prepared round into a Result that must be passed to
`batch.accept(...)`, including on failure. It does not send anything. Returned
unprocessed writes must match a complete issued put/delete payload, comparing
numbers and number sets by parsed Decimal value, including inside maps/lists.
String and binary sets compare by membership, independent of member order.
Action kinds, attribute names and other values must match exactly; duplicates,
changed payloads and
foreign tables are protocol errors. Matching is independent of response order.
Foreign or stale round results conservatively freeze the recipient batch too.

The adapter-local names `BatchWriteError<R>`, `BatchWriteErrorKind`,
`BatchWriteResult<R>` and `BatchWriteAbort::{Cancelled, Deadline}` specialize
the BatchError/BatchRoundAbort sketches below without changing the pure core.
Error `partial` is the core's ordered `BatchWritePartial`; success `actions`
are in original order. Both retain `capacity: Vec<Option<Vec<ConsumedCapacity>>>`
with one entry per issued call. Absent, empty and populated SDK vectors remain
distinct, with table/index details and vector order unchanged; SDK failures,
aborts and abandoned calls contribute `None`, never invented capacity.
`accept` and `finish` return terminal errors in `Arc`, letting the frozen batch
and caller retain the same owned partial without Clone bounds. Repeated failure
access returns the same error; drop/finish the batch to release its ownership
before using `Arc::try_unwrap` to recover owned pending actions. SDK/protocol
failures and explicit aborts call `fail_round`; no indeterminate action is
issued again. Finishing an outstanding call is also conservatively
indeterminate, even if its decoded result was dropped before acceptance.
Every SDK error, including modeled throttling and validation refusals, makes
outstanding writes indeterminate because the adapter never infers "not applied"
from an error.
The second, stacked part of milestone 45 adds a conservative 16 MiB JSON wire
gate at batch construction, separate from the core's 1..=25 action gate and
400 KiB put-item gate. It counts UTF-8 names/strings with JSON escaping,
base64 expansion for binary values/sets, tags, containers, separators and
request options using checked arithmetic. A fixed 128-byte root/options
reservation, 32 bytes per action, and eight bytes per value tag (nine for BOOL
and NULL) may refuse requests near the service boundary. Newline, carriage
return, tab, backspace and form feed use their two-byte JSON escapes; other
control characters use six-byte escapes. Capacity choice is included before
it is selected;
subsequent rounds are subsets and cannot exceed the original bound. This is
a local upper bound for compact DynamoDB JSON, not a measured SDK body size or
remote billing claim. Milestone 46 shares this estimator with expression binding
checks and transaction wire accounting. BatchGet retains its 100-key request
cap; its 16 MiB response cap depends on unknown stored rows and is not a local
request-size gate.

Batch write accepts only `BatchAction::put(&record)?` and
`BatchAction::delete(key)?`. No conditions, updates, version-controlled
records, or return-old-item options are allowed: the AWS batch API cannot enforce
them. A versioned record must use a conditional single write or transaction.

`finish` on exhausted state returns `BatchError::BudgetExhausted` carrying
all successes, missing keys, pending keys/actions, capacities by round, and
issued-call count. Service/transport and decode errors also preserve partial
progress. The caller may explicitly create a new batch from pending work with
a new budget. tabulary never resubmits a failed SDK call or retries an ambiguous
write, and it never silently discards successful work.

When `accept` receives an SDK-level failure, the round's actions are
`Indeterminate` for writes and uncompleted for reads in the returned partial
error. If the caller abandons an outstanding call without a result, it must use
`abort_round(BatchRoundAbort::Cancelled)` or
`abort_round(BatchRoundAbort::Deadline)` to settle the state. This freezes it;
`finish` then returns the abort cause and partial state. Neither path treats
abandoned or timed-out writes as safe retries.

## 11. Single-write failures and mixed-record transactions

### 11.1 One failure vocabulary

```rust,ignore
#[non_exhaustive]
pub enum FailureKind {
    ConditionFailed,
    TransactionConflict,
    ReplicatedWriteConflict,
    ProvisionedThroughputExceeded,
    Throttled,
    RequestLimitExceeded,
    ItemCollectionSizeLimitExceeded,
    Validation,
    NoFailureReported,
    Unavailable,
    UnknownServiceCode(String),
}

pub enum OldItem<R> {
    NotRequested,
    Unavailable,
    Decoded(R),
    Invalid { error: DecodeError, raw: Redacted<Item> },
}

pub struct Failure<R> {
    pub kind: FailureKind,
    pub old: OldItem<R>,
}
```

Single writes return
`WriteError<R, E>::Refused { failure: Failure<R>, source: Box<SdkError<E>> }`
for modeled conditional, conflict, and throttle refusals. `E` is the exact SDK
operation error type. Unclassified service errors and construction, timeout,
dispatch, and response failures retain the typed original under
`WriteError::Sdk`; these are never classified by message text.
Decode faults on successful returned items have a separate `WriteError::Decode`
with the raw output and capacity.
That error means the write committed but its returned item failed decoding;
it does not turn a committed mutation into a refused one.

`.on_condition_failure(OldItemMode::All)` requests
ReturnValuesOnConditionCheckFailure = ALL_OLD. Default `None` makes no request.
When the service provides an old item, strictly decode it as the record, not a
projection. Malformed old data produces `OldItem::Invalid` while keeping the
original failure kind. A missing old item is `Unavailable`, even when requested;
it can mean the item did not exist or the endpoint did not supply it.
It must not fabricate an empty record or erase the condition failure.
[The SDK's modeled exception](https://docs.rs/aws-sdk-dynamodb/latest/aws_sdk_dynamodb/types/error/struct.ConditionalCheckFailedException.html)
provides the old item.

Errors keep SDK metadata and original source errors accessible. Their own
Debug/Display output includes only structured codes and positions.
No code path parses an exception's message, Display, or source text.
Matching the documented machine-readable CancellationReason.Code is permitted.

Milestone 26 exposes `WriteError::<R, E>::from_sdk(error, old_item_mode)` for
the SDK's put, delete, and update error types, with `R: Record + Document`.
It matches only modeled conditional-check, transaction-conflict,
replicated-write-conflict, provisioned-throughput, throttling, and request-limit
variants. A separate `ReplicatedWriteConflict` kind preserves the SDK distinction
between replicated and transaction conflicts. Other service
variants (including item-collection size failures) remain `Sdk`; the broader
failure vocabulary also serves later transaction cancellation decoding.
`OldItem::decode` returns `NotRequested` for mode `None`, even if an endpoint
supplies unsolicited data; the original SDK source still retains that data.
`Redacted<T>::expose()` and `into_inner()` provide explicit raw access. Formatting
never reveals decoded old records or raw payloads and requires no `R: Debug`.
`WriteDecodeError` retains the returned attributes as `raw: Redacted<Item>`,
the structured `source`, and unchanged `capacity` and `item_collection_metrics`.
The operation adapters populate this committed-write error from successful SDK
outputs; refusal classification never constructs it.

Milestone 27 adds `Table<R>::put(&record)` for `R: Record + Document`.
The builder borrows the record until `build()`, which encodes through its
document codec, validates the conservative 400 KiB item bound, and owns the
result in `PreparedPut<R>` (`PreparedRequest<PutItemInput, R>`). Conditions
are validated and rendered at build; repeated `condition` calls AND their
predicates. Refusal decoding derives `OldItemMode`
from the immutable input's failure-return option and uses the shared classifier.
The registered `01_simple.rs` test uses `BilledSize::of_item` directly;
`Record::billed_size()` is also available after the milestone 49 contract review.

### 11.2 Composing a transaction

`tabulary::transaction((action_a, action_b, action_c))` accepts prepared
transaction actions from any record types and table bindings. Put, update, and
delete builders expose `into_tx()?`. Table `check(key, condition)` does too.
This conversion freezes each action's inline condition and update AST.
Standalone ReturnValues and per-action capacity options are rejected; the
transaction owns capacity selection. Conditions can join optimistic guards;
they are never relocated to a separate same-item check.

Milestone 39 is split at the action/request boundary. Its first part adds
`TxAction<R>` and `CheckBuilder<R, K>`, with `into_tx()` on both single-write
builders and their prepared requests. Conversion moves the existing encoded
item/key and rendered expressions without running codecs again, retaining the
record decoder for milestone 40. Non-`None` success returns, capacity, and
collection-metrics selections are refused rather than silently dropped;
transaction-wide accounting belongs to the request builder in the second part.
Default/explicit `None` selections remain valid. `check` reuses the delete
key/condition preparation path and becomes only a `ConditionCheck` action.

```rust,ignore
let request = tabulary::transaction((
    invoices.put(&invoice).expect_new()
        .condition(InvoiceAttrs::INVOICE_REF.not_exists()).into_tx()?,
    inventory.update(stock_key)
        .changes(Update::new().and(StockAttrs::ON_HAND.increment(-1)))
        .condition(StockAttrs::ON_HAND.ge(1)).into_tx()?,
    sessions.check(session_key, SessionAttrs::USABLE.eq(true)).into_tx()?,
))
.client_request_token(token)
.capacity(Capacity::Total)
.build()?;
let result = request.apply(client.transact_write_items()).send().await;
let outcome = request.decode(result);
```

All four action kinds support `on_condition_failure(All)`.
The transaction request is one `TransactWriteItemsInput` with request-order
actions. A supplied idempotency token is passed through and validated; tabulary
does not generate one, retry with one, or reconcile ambiguous results.
The caller must keep the token and input stable for service idempotency.

The builder validates 1..=100 actions, duplicate table/key targets, known item
size bounds, and the known portion of the aggregate limit. DynamoDB's 4 MB
aggregate limit also depends on existing items for deletes, updates, and checks;
the builder cannot prove it locally. It never splits a transaction.
Tables must be in the same account and region; ARN bindings permit local
checks, while a single supplied client and service validation handle names.
See [TransactWriteItems](https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/API_TransactWriteItems.html).

Milestone 39's second part supplies `transaction`, `TransactionBuilder<A>`, and
`PreparedTransaction<A>`. It supports single actions and flat tuples of arity
one through twelve, retaining the typed action shape (available through
`actions()`) for milestone 40. Nested tuples and homogeneous vectors belong to
milestone 41; the final request validator already enforces the 1..=100 cap.
Each action retains its own `expressions()`; the transaction has no shared
expression map. `apply` copies every SDK input field without execution.
Caller tokens have 1..=36 characters and are passed through unchanged; omitted
tokens remain omitted. Capacity and collection metrics are request-wide.
Duplicate detection compares physical key names and scalar values, normalizing
numeric spelling privately and treating a table name and its ARN as aliases.
Explicit DynamoDB ARNs must agree on partition, region, and account. The known
aggregate contribution checked here is the conservative size of put items;
existing item sizes remain service checks. Decoding is added by milestone 40 below.

Milestone 46 adds `PreparedTransaction::size() -> TransactionSize`, reporting
`known_put_item_bytes`, `unknown_item_count`, and `conservative_wire_bytes`.
The first is the conservative put contribution checked against 4 MiB; the
second counts updates, deletes and checks whose stored rows cannot be sized
locally. The wire estimate counts all action payloads, tables, expressions,
bindings, the optional caller token, and root/options reservations with checked
arithmetic. It reuses BatchWrite's JSON value estimator, with 128 root/options
bytes and 192 wrapper/options bytes per transaction action. There is no separate
documented 4 MiB wire cap: base64-expanded JSON may exceed 4 MiB while known item
bytes fit. Passing local checks does not certify the stored-row aggregate or an
update's resulting 400 KiB item bound. Update builders never infer a resulting
item size from just the key and expression values. Remote wire measurements
and billing equivalence remain unverified.

### 11.3 Typed cancellation results without reflection

`TxActions` has an associated `Reasons` type. A
`TxAction<R>` maps to `Failure<R>`; a tuple maps to the corresponding tuple,
recursively. The prepared transaction's decoder returns
`Result<TxResult, TxError<A::Reasons>>`.
Thus the preceding request yields cancellation reasons typed as
`(Failure<Invoice>, Failure<Stock>, Failure<Session>)`.

Support tuples of arity one through twelve and nested tuples.
`Vec<TxAction<R>>` supports homogeneous runtime-length work. Nesting a vector
and tuples supports mixed transactions up to the service action cap without
requiring a hundred tuple implementations. Flattening order is depth-first,
left-to-right; decoding rebuilds that exact shape. There is no erased record
object, unchecked token, or downcast.

Milestone 41 extends the same sealed `TxActions` implementations recursively:
each tuple member may itself be an action, tuple, or `Vec<TxAction<R>>`.
A vector maps to `Vec<Failure<R>>` with its original length and order; vectors
of tuples or mixed record types are not part of this surface. Empty vectors
contribute zero request actions and zero reasons, including inside tuples.
Only the final flattened request must be nonempty and contain at most 100
actions; duplicate-target and size checks run over that same flattened order.
Missing or malformed reason vectors still rebuild every tuple and vector,
with `Unavailable` failures for each leaf and no inferred alignment.

`TxError::Cancelled(Cancellation<C>)` contains:

- `reasons: C`, preserving every action's typed old item.
- `completeness: Complete | Missing | MalformedLength { expected, actual }`.
- The original `SdkError<TransactWriteItemsError>` and accessible raw reasons.

With a complete reason vector, `ConditionalCheckFailed`,
`TransactionConflict`, `ProvisionedThroughputExceeded`, `ThrottlingError`,
`ItemCollectionSizeLimitExceeded`, and `ValidationError` map to their
respective FailureKind values. A null code or the explicit code `None` means
`NoFailureReported`, not that the transaction committed. Unknown codes remain
`UnknownServiceCode`. Old items decode independently for each action.
The [SDK reason model](https://docs.rs/aws-sdk-dynamodb/latest/aws_sdk_dynamodb/types/struct.CancellationReason.html)
specifies request ordering and null-code entries.

If the endpoint supplies no vector, every action has `Unavailable`.
If its length differs, do not guess alignment: every action is `Unavailable`,
retain the raw vector, and mark completeness malformed. Do not parse the message
to reconstruct missing reasons. The entire transaction failed, but its detailed
cause may be unknowable. Non-cancellation SDK failures are `TxError::Sdk`.
Transport failures can have an indeterminate commit outcome.

Milestone 40 supplies `PreparedTransaction::decode` for single actions and flat
tuples of arity one through twelve. Nested tuples and vectors remain milestone
41. `ReasonCompleteness` names the three completeness states above. An absent
SDK vector is `Missing`; a present empty vector is `MalformedLength` for a
nonempty request. Missing or misaligned vectors supply no old items to any
decoder. Each action independently honors its frozen failure-return option:
unrequested old data stays `NotRequested`, even when detailed reasons are
unavailable, and requested but absent old data is `Unavailable`.

`Cancellation<C>` exposes `source: Box<SdkError<TransactWriteItemsError>>` and
`raw_reasons()` preserves absent versus present vectors. `TxError::sdk_error()`
and standard error sources retain the original SDK error. Wrapper formatting
redacts reasons, items, messages, and SDK sources without requiring record
`Debug` implementations. `TxResult` preserves the SDK's optional capacity vector
as `capacity` and optional per-table `item_collection_metrics` unchanged. The
decoder uses each frozen action's stored full-record decoder, with no reflection
or type bounds on the record beyond those already checked during preparation.

The application decides what conflicts, throttles, and failed conditions mean
to its business operation. There is no prioritized summary that discards other
item reasons and no built-in retryability policy.

## 12. Opt-in optimistic locking

`#[tabulary(version)] edit_no: u64` declares a required native numeric version.
No default, optional field, converter, second version, or composite-key version
is allowed. Its handle supports conditions but not general mutation methods.
Decoding a missing or malformed version fails normally.

Every mutation of a versioned record requires an explicit policy:

| Policy | Put | Update | Delete |
| --- | --- | --- | --- |
| `expect_new()` | Guard primary key absence; record must explicitly carry version 1. | Unavailable. | Unavailable. |
| `expect_version(n)` | Guard stored version == n; proposed record must carry n+1. | Guard stored version == n, SET version n+1 in the same expression. | Guard stored version == n, no increment. |
| `unchecked_version()` | Explicit bypass for maintenance. | Explicit bypass; no automatic increment. | Explicit bypass. |

`n+1` uses checked addition; overflow is a build error. No absent-version
compatibility branch is inferred. A missing row does not match expected version
zero. Put never silently rewrites the supplied record's version.
User conditions AND with version guards, and rendered expressions expose both.
Transactions use identical semantics; a version mismatch remains
`ConditionFailed`, with the old item if requested. It is not relabeled as a
version conflict because another user condition might have failed.

There is no local object mutation after a write, no retry with a refreshed
version, and no hidden GetItem. An application constructs the next value or
requests ALL_NEW explicitly.

Milestone 30 represents the version declaration with the existing required,
number-wire `FieldAccess::VersionOnly` descriptor. The derive accepts a written
`u64` or its `core::primitive`/`std::primitive` path, and rejects aliases,
primary-key fields, and sources of primary or secondary composite keys.
Document derives reject version annotations. The metadata-only `Record` trait
and prepared write signatures stay unchanged.

Missing policy on a versioned record, repeated policy calls (even identical
ones), an incorrect proposed version, and successor overflow return
`BuildError::VersionPolicy`. Delete does not compute a successor, so it accepts
`expect_version(u64::MAX)`. An otherwise empty update with `expect_version`
performs the version SET alone; an empty unchecked update is still invalid.
Unversioned writes need no policy; put may use `expect_new()` to guard primary
key absence. `expect_version` and `unchecked_version` require a version field.
Policy expansion runs at build, joins caller conditions with AND, and appends
the version SET before ordinary update validation and shared rendering.
Transaction conversion uses these same prepared inputs; it adds no version
policy of its own.

## 13. Item size and capacity boundaries

`Record::billed_size(&self)` encodes with the same document codec as a put.
It requires `Self: Document` and returns `Result<BilledSize, EncodeError>`.
`BilledSize::of_item(&Item)` sizes an already encoded
item without cloning. It includes physical names, synthetic keys, and only
attributes actually present.

Milestone 09 provides the standalone estimator; milestone 49 adds the
`Record::billed_size` convenience. `of_item`
accepts a borrowed SDK `HashMap<String, AttributeValue>` (the `Item` alias) and
returns `Result<BilledSize, EncodeError>`. Its diagnostic root is
`Item`, with physical attribute/index paths. It validates numeric domain and
document depth, but is not a general schema or set-uniqueness validator.
Invalid numeric syntax/domain returns `NumberRange`; malformed NULL and future
SDK variants use `Custom` codes `invalid_null` and `unsupported_type`, because
the encode fault taxonomy has no dedicated variants for these cases.
`ensure_at_most` returns `BuildError::RequestLimit { max, actual }` on excess,
where `actual` is the conservative count; equality succeeds.

`BilledSize` exposes `estimated_item_bytes()` and
`conservative_item_bytes()`. “Item bytes” means the item size relevant to
capacity rounding and item limits, not JSON wire bytes or total storage billing.
The method name does not promise an exact remote invoice.

The sizing algorithm uses UTF-8 byte lengths for names and strings, raw binary
lengths, one byte for bool/NULL, list/map container overhead and per-child
overhead, and scalar member sums for sets. A top-level item has no map wrapper
overhead. Omitted options contribute nothing. Estimated numbers use normalized
significant digits: ceil(digits/2)+1, with an extra byte reserved for negatives.
The conservative model reserves 21 bytes per valid native number, even zero.
All sums use checked arithmetic. List/map overhead is three bytes per container
and one byte per child; sets have neither overhead. Zero has one significant
digit and no negative sign after normalization. Remote billing equivalence
remains **unverified** until live-service boundary experiments are run.

AWS describes number sizes as approximate, so these are a documented estimate
and conservative model, not a service-certified exact byte count.
`size.ensure_at_most(max_bytes)` uses the conservative count and may reject
items near a boundary that the service would accept. It gives callers a stable,
explicit bound under this model; the service remains authoritative.
[AWS sizing rules](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/CapacityUnitCalculations.html)
are the source for scalar/container overhead and the precision limitation.

The item size check uses the service's 400 KiB item cap. Physical primary and
index key limits have separate checks. Storage billing adds per-item and
feature-specific overhead that this count does not include; GSI copies also
cost storage independently. Wire JSON/base64 expansion is separately counted
for request limits.

No item-size function predicts exact consumed capacity for queries, conditional
checks, transactions, index writes, or updates whose old item is unknown.
Callers use returned consumed capacity for accounting. Single-item unit-rounding
helpers, if added, must require the operation and consistency explicitly.

## 14. Derivable table definitions

`R::table_definition()` derives `TableDefinition<R>` from static schema:
primary key names and scalar types, GSI/LSI definitions, index projections, and
the optional TTL attribute. Only key attributes enter AttributeDefinitions.
Regular fields are not DynamoDB table schema definitions.

```rust,ignore
let definition = Parcel::table_definition();
let create = definition.create_input("parcels_test", Billing::OnDemand)?;
let ttl = definition.ttl_input("parcels_test")?; // Option<UpdateTimeToLiveInput>
```

`create_input` returns the SDK `CreateTableInput`.
`Billing::Provisioned` requires table and every GSI throughput explicitly.
Milestone 37a adds the `Record::table_definition()` default method (with the
`Record` trait in scope), `Billing`, and `Throughput { read, write }`. Provisioned
billing takes `table: Throughput` and `global_indexes: BTreeMap<String,
Throughput>` keyed by physical GSI name. Both unit counts must be positive;
missing or extra GSI entries, including LSI entries, are errors. On-demand
inputs omit throughput. Empty index lists are absent SDK fields.

The input uses deterministic physical-name order for attribute definitions and
indexes, HASH before RANGE for keys, and sorted INCLUDE names. It revalidates
manual schemas as well as derives: key positions, S/N/B types, distinct key
names, required primary keys, index-name collisions, and LSI restrictions.
The initial limits are 20 GSIs, 5 LSIs, 1–20 INCLUDE attributes per index and
100 INCLUDE entries summed over indexes (repeated names in different indexes
count separately). Automatically projected keys cannot be INCLUDE entries.
Table/index identifiers are 3–255 ASCII letters, digits, underscores, hyphens,
or dots; physical key and INCLUDE names are 1–255 UTF-8 bytes. Invalid schema
definitions use `BuildError::IncompatibleIndex`; invalid billing/name settings
use structured `OperationConstraint` codes. These helpers add no dependencies.

The generator checks LSI partition equality, scalar types, index/projection
limits, and collisions. An LSI on a hash-only table is invalid.
Same-table multiple record schemas can use
`TableDefinition::merge(a, b)?`, which requires exact agreement for overlapping
definitions and one compatible TTL setting.
Milestone 37b adds `merge` and TTL metadata/inputs. Merging compares physical
names and S/N/B types, independent of each record's field positions. Primary
keys must agree; disjoint index names are unioned; shared names must agree on
kind, HASH/RANGE keys, and projection (INCLUDE order does not matter). Combined
index/projection limits are checked after deduplication. Any declared field
that shares a merged physical key or TTL name must agree with its wire type.
The merged definition retains the left record's type marker.

TTL absence means unspecified, so it can merge with one declared TTL attribute;
two different declared TTL names conflict. `#[tabulary(ttl)]` adds the field's
position to `RecordSchema::ttl`. It accepts exactly `TtlSeconds` or
`Option<TtlSeconds>`, including aliases; duplicate TTL fields/annotations,
document TTL declarations and flattened TTL payloads are rejected. Unannotated
TTL values do not enable expiry. `ttl_input` returns `None` for an undeclared
TTL, or a separate enable request using its physical wire name. An empty table
identifier is rejected; names and ARNs are otherwise preserved for this SDK
operation. No request disables TTL implicitly.

TTL requires a separate UpdateTimeToLive request after table activation; it is
not a CreateTable field. Definition helpers do not create tables, wait for
activation, enable TTL, change throughput, or reconcile existing schemas.
Tests issue each request explicitly and poll with their own bounded wait.
No production migrations or schema synchronization are included.

## 15. Derive expansion and diagnostics

Milestone 11a implements named-field `Record` derives with native codecs, all
four rename rules, `generated_prefix`, static handles/schema, and owned keys.
The [checked expansion fixture](../crates/tabulary-derive/tests/fixtures/record.expanded.tokens)
is the actual emitted token stream for the adjacent `record.rs` input, checked
for token equality and compiled as an integration-test module. It takes
precedence over the completed-surface sketch below for this stage.

The merged metadata-only `Record` trait stays unchanged. For now the derive
emits inherent `to_item`, `from_item`, `key`, and `primary` methods; `RecordKey`
provides key encode/decode, accepting a full item's extra fields on key decode.
Milestone 11b adds missing-only explicit defaults and the configurable unknown
policy: full record decode rejects unknown fields in sorted order unless
`unknown = "allow"` is declared. Its compile-fail snapshots cover duplicate wire
names at both declarations, optional/non-scalar primary keys, key defaults,
invalid/duplicate policy options, and invalid default options/signatures.
Milestone 12 adds the public `Document` map trait and generates `Document` and
map-wire `Value` implementations for both derives. `Document` emits only its
`<Name>Attrs` peer and codecs; it rejects key annotations. Both derives share
naming, explicit defaults, and unknown-field rules. Record item methods delegate
to its document codec, retaining key validation when embedded as an `M`.
The metadata-only `Record` trait remains unchanged. Milestone 23 adds the
query-facing primary and index interfaces described in §9.1.

Generic peers retain type and const parameters and caller predicates. Generated
implementations require field codecs and owned key cloning, without requiring
non-key values to implement `Clone`. Lifetime parameters and references inside
field types are rejected. Both derives accept the explicit `crate` path override.
The top-level map is depth zero; each nested `M` or `L`, including an empty
container, consumes one of 32 levels. Scalars and sets consume no levels.
Nested codecs preserve the outer record, current document, and physical path.

Physical keys validate S/N/B categories at compile time and nonempty/byte limits
on construction, encoding, and decoding. The existing decode taxonomy has no
empty-key or key-size variants: these decode failures use
`KeyFormat(InvalidLength)` at the physical attribute; encoding retains
`EmptyKey` or `KeySize`. The native codec reports numeric-domain failures first.

The checked expansion fixture above is the source of truth for generated code.
`expand::tests::expansion_matches_fixture` checks its tokens and
`crates/tabulary/tests/record_derive.rs` compiles the fixture as a module.
Generated code uses reserved macro/runtime hooks. These hooks are not application
examples or a supported manual-construction API. The former semantic sketch was
removed because it advertised nonexistent MapWriter/MapReader/KeyWriter/KeyReader
helpers, a six-argument schema helper, and a richer Record trait than implemented.

Generated keys implement `Clone`, without adding `Clone`, `Debug`, or `Default`
to the record. Key field types must support owned cloning; keys do not require
Rust Eq. Identity compares canonical physical values. The native Value codecs
own scalar conversion.

Derives parse attributes into a small checked schema IR, then generate code.
Reject duplicates, unsupported combinations, missing names/keys, bad templates,
non-scalar keys, invalid versions/TTL, ambiguous variants, and cross-owner
projections before token emission where possible. Use `syn::Error::new_spanned`
on the source field or attribute, combine independent errors, and use
`quote_spanned!` for codec/type bound checks. Do not panic on user syntax.
Raw Rust identifiers map to unprefixed names under naming rules.

The generated path is `::tabulary` by default;
`#[tabulary(crate = "::renamed_dependency")]` handles dependency renaming
without adding a manifest-inspection dependency. Generic structs are supported
when their field types satisfy the generated codec/key bounds. References,
borrowed string fields, and lifetime-bearing record types are excluded in the
first version; owned decoded values avoid response lifetime traps.

Generated constructors are doc-hidden runtime helpers rather than unsafe
backdoors. At build, validate descriptors against the owner's static schema,
so forged descriptors cannot bypass immutable-key, version, or index checks.
Supported codec, document, and record traits remain manually implementable.
Public visibility of reserved expansion hooks does not make manual typed-handle
construction supported. This is type discipline, not a security boundary against
deliberately dishonest trait implementations.

## 16. Test contract and CI

CI runs fmt, check, clippy with warnings denied, and workspace tests on the MSRV.
The compatibility matrix runs every workspace test target and doctest against the
exact SDK floor and current MSRV-compatible dependencies, in both feature modes.
Only derive macro re-exports depend on the `derive` feature; runtime implementations
have the same meaning in both modes. Derive-dependent tests are feature-gated;
manual codec, schema, expression, SDK, and state tests still run without it.
`scripts/check_sdk_features.py` checks the normal/build graph in both modes.
The endpoint runner inventories every ignored endpoint test before running it and
requires all inventoried tests to pass. There are no ignored tests outside that
suite. Deterministic SDK fixtures cover responses that cannot be induced reliably
offline. The required evidence is:

| Layer | Required evidence |
| --- | --- |
| Unit codecs | Every supported type round trip; missing/wrong-type/malformed distinctions; optional NULL rejection; integer and decimal boundaries; float range/inexact/negative-zero; binary length; normalized sets; explicit defaults and unknown policy. |
| Unit schemas | Composite inverse parsing, mismatched duplicates, separator ambiguity, sparse membership, index reindex dependencies, TTL/version validity, and table-definition SDK input snapshots. |
| Unit expressions | Exact text and both maps; reserved names, dots as literal segments, deterministic aliases, shared allocator, all operators, nested indexes, action overlaps, protected keys, converter constants, and unknown SDK variants. |
| Compile fail | `trybuild` source/diagnostic snapshots for bad derives and expressions. Wrong owner/type/codec, non-key query operand, missing consistency, illegal version mutation, missing fields, and invalid projection must fail at a useful span. |
| SDK fixtures | Decode modeled errors and synthetic outputs: conditional old items, malformed old data, every cancellation code, absent/misaligned vectors, capacity, empty filtered pages, reordered batch results, unprocessed work, and transport ambiguity. |
| Endpoint integration | Actual typed put/get/update/delete, strong reads, nested values, GSI sparsity, projection, parallel scan, pagination, batch partial state, transactions with mixed types, conditional old items, optimistic races, and table/TTL request shapes. |

Endpoint tests read **`TABULARY_DYNAMODB_ENDPOINT`** and require an explicit URL.
They build a client with the usual AWS region/credential configuration; the
test harness disables SDK retries when asserting exact call counts.
When the variable is absent, ignored integration tests are unexecuted, not
passed integration evidence. `cargo test --test endpoint -- --ignored` without
the variable fails immediately. The normal unit test run needs no endpoint.

An implementation CI job runs dynoxide, a DynamoDB emulator, at an explicitly
pinned revision, sets the endpoint to its loopback port, uses dummy credentials and a region,
then runs `cargo test -p tabulary --test endpoint -- --ignored`.
It performs a bounded readiness check and unique per-test table naming, and
cleans up only tables it created. Index/TTL activation polling has a deadline.
Tests never discover and delete arbitrary tables.

The same suite runs against real DynamoDB when given its regional HTTPS endpoint
and valid credentials with create/use/delete permissions for isolated test
tables. This is opt-in and incurs service work; no production table names are
compiled into tests. Service limitations are marked per capability rather than
weakening assertions or claiming emulator equivalence.
TTL enablement and stored TTL value are tested without waiting for physical
expiry. Deterministic fixtures cover throttles and omitted cancellation vectors
that cannot be induced reliably on every endpoint.

Each example starts as syntax-checked design source. Its owning milestone must
register it as a Cargo example or compile test and make it type-check against the
real API; no temporary mock crate may satisfy that gate.
Wire-size and conservative numeric-size tests additionally need a documented
live-service boundary experiment before claims of exact billing equivalence.

### 16.1 Public contract review

The public surface includes these inspectable/supporting APIs in addition to the
operations described above:

| Surface | Contract |
| --- | --- |
| `condition::{Operator, Operand, ConditionNode, Size}` | Read-only built AST inspection and the typed `size()` operand. `root()` borrows and `into_root()` consumes the condition. Inspecting `Operand::Value` explicitly exposes its raw SDK constant. |
| `update::{Action, SetOperand, SetExpression, SetNode, ActionNode}`, `Path`, `MapKey`, `Element` | Typed unbuilt actions/operands and built tree inspection. `Element` marks list-element presence. Constructed AST nodes cannot be injected into builders. |
| `FieldDescriptor`, `KeyDefinition`, `RecordSchema`, `IndexDefinition`, `IndexKind`, `IndexProjection` | Public static metadata for manual schemas. `RecordSchema::attribute_definitions()` returns physical key definitions; `with_indexes` and `with_ttl` are doc-hidden derive helpers. Version metadata uses `FieldAccess::VersionOnly`, not a schema field. |
| `EncodeContext`, `DecodeContext`, `AttributePath`, `PathSegment` | Manual codec traversal via `new`, `at`, `in_document`, `enter_container`, `error`, `segments`, and `attribute`; child contexts preserve their parent. |
| `Cursor::last_evaluated_key`, `PageMetadata::last_evaluated_key` | Explicit raw continuation-key access; Debug remains redacted. Page metadata retains SDK counts. |
| `Batch*Progress`, `Batch*Chunk`, `Batch*Entry`, `Batch*State`, `Batch*Complete`, `Batch*Incomplete`, `Batch*Partial` | Public pure progress cores and ordered results in section 10. `positions`, `keys`, `entries`, and `calls_issued` inspect state without issuing work. Adapter round/result/error names specialize each API. `BatchAction::key/item` explicitly expose frozen raw values. |
| `Redacted::new/expose/into_inner`, `OldItem::decode`, `Table::name`, `BatchBudget::max_calls` | Explicit wrapping/raw access, standalone old-record decoding, binding identifier and budget inspection. |

Reserved public hooks also exist outside `__private`: doc-hidden
`projection::{ProjectionSource, SourceField, Handle, NestedValue}` and helper
functions, `Projection::project_source`, `key_component::take`, and
`TxActions::decode_reasons`. Generated doc-hidden `__Tabulary*ProjectionParts`
carriers, `*PartitionCodec`/`*SortCodec` peers, and `*PrefixParts` traits also
support cross-crate expansion. These names need public visibility to appear in
associated types or generated bounds; they are not independent application APIs.
Typed projections and their `ProjectionSource` hooks are generated only; manual
projection implementations are outside the supported API.
The seven worked examples and README use no `__private` items.

The review found a larger gap in the intended contract that remains unimplemented:

- Section 4.1's `validate = ...` record hook and `ValidationFault` type do not exist;
  the derive rejects that option. There is no acceptance test for that promised
  hook or the status-dependent sparse-membership validation that relies on it.

This gap requires follow-up work; the final review does not claim the whole
design is implemented. The metadata-only Record boundary and missing `Primary<R>`
alias are stated in section 2. The small `Record::billed_size()` omission is fixed
with a provided method constrained by `Document`, as described in section 13.

### 16.2 Real-service evidence still needed

No real DynamoDB run has verified extreme numeric continuation-key spellings
and therefore acceptance of the 192-byte cursor number bound. The 2 MiB
expression-binding cap uses a conservative JSON measurement rather than an
observed service boundary. Transaction aggregation assumes binary megabytes
(4 MiB) for the service's documented 4 MB limit. TTL request shapes and stored
values have fixtures/emulator coverage, but real-service TTL activation timing
and physical expiry remain unverified. Conservative item sizing also lacks
remote billing-equivalence evidence. None of these offline gates claims exact
service equivalence.

## 17. Alternatives rejected

| Alternative | Reason for rejection |
| --- | --- |
| Serde-only derive with independent expression-name constants | Two schemas can drift; dynamic deserialization alone does not enforce the specified key, path, and update invariants. An optional Serde adapter can be added later. |
| Client-owning repository or generated model client | Hides execution and complicates caller transport, region, middleware, and retry control. |
| Implicit defaults and “wrong type means None” | Turn stored corruption into an apparently valid record or an authorization decision. |
| Arbitrary raw expression paths mixed into typed updates | Escape schema, converter, and immutable-key checks. The SDK already serves raw use cases. |
| Decode projections into the full record | Forces silent defaults for unselected required fields. |
| Erased transaction actions with Any old-item downcasts | Move type mistakes to runtime and weaken request/reason alignment. Typed tuple/vector shapes are sufficient. |
| Silent automatic batch retries or splitting | Hide calls, budgets, and partial work. Explicit bounded state makes every submission visible. |
| Implicit lossy number/string fallback | Changes comparison, arithmetic, key ordering, and indexes based on runtime values. Reject or declare a converter. |
| Default full-range u128 as S | Surprises numeric users and makes ADD unavailable by default. Keep native N with checked range and an explicit full-range string converter. |
| Permissive unknown fields everywhere | Hides misspelled names and inactive variant payloads. Opt in to allowance per type when additive writers require it. |
| Automatic schema create/update or version repair | Crosses the non-goal boundary into migrations and hidden calls. |

## 18. Open questions and recommendations

These are release experiments, not unresolved instructions for the first
implementation. Use the recommendations below until evidence justifies an API
revision. A change must update this contract and examples together.

| Question | Recommendation for the first version | Evidence needed to revisit |
| --- | --- | --- |
| How ergonomic are nested tuple cancellation results near the transaction cap? | Implement arity twelve plus nesting and homogeneous vectors; keep inference examples. | Compile representative large compositions and measure diagnostics/type-check time. Add a tuple macro only if necessary. |
| Does deny-unknown by default make additive rollouts too cumbersome? | Keep deny as the integrity-first default and expose allow per declaration. | Trials with multiple independently deployed writers; do not weaken inactive-variant checks. |
| Should the accepted u128 domain be encoded in a newtype? | Native u128 fields reject nonrepresentable N; provide u128_string explicitly. | Consumer feedback on late encode failures. A checked DynamoNumber<T> can supplement the API. |
| How close is the conservative numeric size model to boundary behavior? | Expose both estimate and conservative count; never advertise exact billing. | Real service experiments at capacity and item-size boundaries, including negative numbers and nested collections. |
| Should cursors be stable across library major releases or signed by the library? | Version V1 bytes; reject unknown versions; leave authentication to the application. | A concrete cross-release resumption requirement and a key-management design. |
| Should views be allowed to fetch missing LSI attributes automatically? | Reject nonprojected attributes on all index views. | Measured demand for explicit base-table-fetch semantics and capacity visibility. |

No implementation worker needs to choose a retry policy, unknown-field default,
numeric fallback, discriminator cleanup rule, or transaction error representation;
those choices are fixed above.

## 19. Requirement map

| Requirement | Contract section |
| --- | --- |
| Struct derive, names declared once, constants/handles | 2 |
| Field/composite keys and inverse parse | 3.1 |
| Secondary and sparse indexes | 3.2–3.3 |
| Scalars, exact numbers, binary, sets, lists, maps, Option/NULL | 4 |
| TTL/timestamps, converters, explicit defaults, secret redaction | 4.1–4.2 |
| Unit and discriminator-selected payload enums | 5 |
| Strict typed faults, full paths, unknown attributes | 6 |
| Typed conditions, updates, nested paths, inspectable expressions | 7 |
| Caller-owned SDK client and explicit consistency | 8 |
| Get, put, delete, update, consumed capacity, no hidden calls/retries | 8 |
| Query key schema, pagination, projections, parallel scan | 9 |
| Batch get/write and bounded visible unprocessed submissions | 10 |
| Conditional failures, transactions, item reasons and typed old records | 11 |
| Opt-in optimistic versions | 12 |
| Billed item sizing and numerical precision of the estimate | 13 |
| Test table/index/TTL definitions | 14 |
| Proc-macro expansion and field-focused diagnostics | 15 |
| Unit, compile-fail, endpoint integration, emulator CI and real endpoint | 16 |
| Crates, dependencies, edition, MSRV, license, unsafe ban | 1.2 |
| Non-goals and rejected alternatives | 1 and 17 |
| Open questions with implementation recommendations | 18 |
