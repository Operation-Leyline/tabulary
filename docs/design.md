# tabulary design

Status: **design**, not an implemented API. This document is the implementation
contract. The empty crates compile; the proposed API examples do not yet
type-check against them. [Milestones](milestones.md) define when each example must
become a compiling test.

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
`#![forbid(unsafe_code)]`. The skeleton has no third-party dependencies.
Implementation adds `aws-sdk-dynamodb = "1.130.0"` with default features
disabled, plus the optional derive crate. The runtime's default feature set is
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
`Primary<R>` is only a convenience alias for `<R as Record>::Primary`, including
when used as a cursor marker. This avoids Rust's foreign inherent-impl restriction.

### 2.1 Runtime traits

These are signature sketches, not code already present in the empty library.

```rust,ignore
pub type Item = std::collections::HashMap<String, aws_sdk_dynamodb::types::AttributeValue>;

pub trait Document: Sized {
    const NAME: &'static str;
    fn encode_map(&self, cx: &mut EncodeContext) -> Result<Item, EncodeError>;
    fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError>;
}

pub trait Record: Document {
    type Key: RecordKey<Self>;
    type Primary: PrimarySchema<Self>;
    const SCHEMA: RecordSchema;
    fn key(&self) -> Result<Self::Key, EncodeError>;
    fn primary() -> Self::Primary;
    fn to_item(&self) -> Result<Item, EncodeError>; // default delegates to Document
    fn from_item(item: &Item) -> Result<Self, DecodeError>; // same
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
Manual implementations of these public traits are supported. Reserved
`__private` macro helpers carry no semver guarantee.

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
Both stages reject `sparse` and optional index inputs explicitly until milestone
17 supplies sparse encode/decode invariants. Key-condition builders arrive in
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

`update.transition(DeliveryAttrs::STAGE, new_stage, expected_tag)` adds a
condition for the expected tag, SETs the new tag and payload, and REMOVEs the
union of old-only payload attributes. It replaces the entire variant value, so
stale fields cannot survive a status change. Common-field updates can join the
same request. Conditional old-item decode uses the old discriminator.

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
`Document::FIELDS` for descriptor validation; manual document codecs must provide
this metadata to support `field` (the default empty slice refuses traversal).
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
substitution checks belong to operation builders and milestone 46.

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

## 8. Prepared operations and SDK integration

`Table::<R>::new(table_name)` validates a nonempty table name or ARN and returns
a binding. It does not describe or create the table. Record definitions never
embed deployment table names.

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
| `query(key_condition, consistency)` | `QueryInput` | One `Page<P, Cursor<R, Primary<R>>>`. |
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

`.return_item(ReturnItem::None | AllOld | AllNew)` is operation-specific.
Put/delete support None or AllOld; update additionally supports AllNew.
UPDATED_OLD/UPDATED_NEW are excluded in the first API because they are partial
shapes and cannot decode into a full record. A later API can add typed update
projections. No read follows a write to obtain return values.

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

Query builders accept `filter(Condition<R>)`, `limit(NonZeroU32)`,
`order(QueryOrder::Ascending | Descending)`, `select::<P>()`, and
`resume(Cursor<R, I>)`. Key fields are rejected in query filters; express
them in the key condition. Scan filters may include key fields.
Filters act after service evaluation and do not reduce the read work. A limit
is an evaluated-item limit per call, not a requested count of matching records.

`Page` contains `items`, `next`, `count`, `scanned_count`, and
`capacity`. Empty items with a nonempty last-evaluated key still produce
`Some(cursor)`. Only an absent or empty last-evaluated-key map means end.
There is no automatic paginator or collection loop. Pages and scans are not
snapshot isolation, even when each read is strong.

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

Opaque does not mean encrypted or authenticated. Tokens contain key and filter
values. Applications exposing them outside a trusted boundary must authenticate
and authorize them, or store them server-side. Matching a cursor is never access
control. The library does not fetch state, sign tokens, or assign tenancy.

### 9.4 Parallel scans

`ScanSegment::new(segment, total)` requires a positive total and
`segment < total`, within DynamoDB's supported segment range.
`table.scan(consistency).segment(segment).resume(cursor).build()` prepares one
page of one segment. An unsegmented scan has a distinct cursor binding.
Changing total or segment rejects the old cursor. Each segment keeps its own
cursor; the application schedules concurrent calls and bounds memory.
There is no automatic fan-out, merge order, or cross-segment progress storage.

## 10. Batches and bounded unprocessed work

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

### 11.2 Composing a transaction

`tabulary::transaction((action_a, action_b, action_c))` accepts prepared
transaction actions from any record types and table bindings. Put, update, and
delete builders expose `into_tx()?`. Table `check(key, condition)` does too.
This conversion freezes each action's inline condition and update AST.
Standalone ReturnValues and per-action capacity options are rejected; the
transaction owns capacity selection. Conditions can join optimistic guards;
they are never relocated to a separate same-item check.

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

## 13. Item size and capacity boundaries

`Record::billed_size(&self) -> Result<BilledSize, EncodeError>` encodes with
the same codecs as a put. `BilledSize::of_item(&Item)` sizes an already encoded
item without cloning. It includes physical names, synthetic keys, and only
attributes actually present.

Milestone 09 provides the standalone estimator; the `Record::billed_size`
convenience method follows when record encoding is introduced. `of_item`
accepts a borrowed SDK `HashMap<String, AttributeValue>` (the planned `Item`
alias) and returns `Result<BilledSize, EncodeError>`. Its diagnostic root is
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
The generator checks LSI partition equality, scalar types, index/projection
limits, and collisions. An LSI on a hash-only table is invalid.
Same-table multiple record schemas can use
`TableDefinition::merge(a, b)?`, which requires exact agreement for overlapping
definitions and one compatible TTL setting.

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
The metadata-only `Record` trait remains unchanged; query-facing primary
interfaces belong to milestone 23. Primary/index peers remain markers.

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

This is the semantic expansion for the three-field Order in the README.
Generated boilerplate is shown using runtime helpers to keep it legible;
the helpers below are part of the planned macro/runtime contract. No
`todo!()` implementation is being passed off as a working derive.

```rust,ignore
struct OrderAttrs;
impl OrderAttrs {
    const ORDER_REF: tabulary::Attr<Order, String, tabulary::Required, tabulary::KeyOnly> =
        tabulary::__private::field("order_ref", 0);
    const LINE_NO: tabulary::Attr<Order, u64, tabulary::Required, tabulary::KeyOnly> =
        tabulary::__private::field("line_no", 1);
    const QUANTITY: tabulary::Attr<Order, u64> =
        tabulary::__private::field("quantity", 2);
}

struct OrderIndexes;
#[derive(Clone, Copy)]
struct OrderPrimary;

impl tabulary::PrimarySchema<Order> for OrderPrimary {
    type Partition = String;
    type PartitionCodec = tabulary::Native<String>;
    type Sort = tabulary::SortKey<u64>;
    const DEFINITION: tabulary::KeyDefinition = tabulary::__private::keys(0, Some(1));
}

impl OrderPrimary {
    fn partition(self, order_ref: String) -> tabulary::KeyCondition<Order, Self> {
        tabulary::__private::partition(self, order_ref)
    }
}

struct OrderKey {
    order_ref: String,
    line_no: u64,
}

impl OrderKey {
    fn new(order_ref: String, line_no: u64) -> Result<Self, tabulary::EncodeError> {
        let key = Self { order_ref, line_no };
        tabulary::__private::validate_key::<Order>(&key)?;
        Ok(key)
    }
}

impl tabulary::RecordKey<Order> for OrderKey {
    fn encode_key(&self) -> Result<tabulary::Item, tabulary::EncodeError> {
        let mut writer = tabulary::__private::KeyWriter::new::<Order>();
        writer.partition(OrderAttrs::ORDER_REF, &self.order_ref)?;
        writer.sort(OrderAttrs::LINE_NO, &self.line_no)?;
        writer.finish()
    }
    fn decode_key(item: &tabulary::Item) -> Result<Self, tabulary::DecodeError> {
        let mut reader = tabulary::__private::KeyReader::new::<Order>(item);
        let key = Self {
            order_ref: reader.partition(OrderAttrs::ORDER_REF)?,
            line_no: reader.sort(OrderAttrs::LINE_NO)?,
        };
        reader.finish()?;
        Ok(key)
    }
}

impl tabulary::Document for Order {
    const NAME: &'static str = "Order";

    fn encode_map(&self, cx: &mut tabulary::EncodeContext)
        -> Result<tabulary::Item, tabulary::EncodeError>
    {
        let mut w = tabulary::__private::MapWriter::new(cx);
        w.required(OrderAttrs::ORDER_REF, &self.order_ref)?;
        w.required(OrderAttrs::LINE_NO, &self.line_no)?;
        w.required(OrderAttrs::QUANTITY, &self.quantity)?;
        w.finish()
    }

    fn decode_map(item: &tabulary::Item, cx: &mut tabulary::DecodeContext)
        -> Result<Self, tabulary::DecodeError>
    {
        let mut r = tabulary::__private::MapReader::new(item, cx);
        let value = Self {
            order_ref: r.required(OrderAttrs::ORDER_REF)?,
            line_no: r.required(OrderAttrs::LINE_NO)?,
            quantity: r.required(OrderAttrs::QUANTITY)?,
        };
        r.finish(tabulary::UnknownAttributes::Deny)?;
        Ok(value)
    }
}

impl tabulary::Record for Order {
    type Key = OrderKey;
    type Primary = OrderPrimary;
    const SCHEMA: tabulary::RecordSchema = tabulary::__private::schema(
        "Order",
        &[
            OrderAttrs::ORDER_REF.descriptor(),
            OrderAttrs::LINE_NO.descriptor(),
            OrderAttrs::QUANTITY.descriptor(),
        ],
        tabulary::__private::keys(0, Some(1)),
        &[], // indexes
        None, // TTL
        None, // version
    );

    fn key(&self) -> Result<OrderKey, tabulary::EncodeError> {
        OrderKey::new(self.order_ref.clone(), self.line_no)
    }
    fn primary() -> Self::Primary {
        OrderPrimary
    }
}

impl tabulary::Value for Order {
    type Wire = tabulary::wire::Map;
    fn encode_value(&self, cx: &mut tabulary::EncodeContext)
        -> Result<aws_sdk_dynamodb::types::AttributeValue, tabulary::EncodeError>
    {
        <Self as tabulary::Document>::encode_map(self, cx)
            .map(aws_sdk_dynamodb::types::AttributeValue::M)
    }
    fn decode_value(value: &aws_sdk_dynamodb::types::AttributeValue,
        cx: &mut tabulary::DecodeContext) -> Result<Self, tabulary::DecodeError>
    {
        tabulary::__private::decode_document(value, cx)
    }
}
```

`Record` default methods implement to_item/from_item/billed_size/table_definition
using these generated pieces. The expansion additionally emits Clone for
the key, direct primary-key query component trait implementations, and index
builders when declared. It does not add Clone, Debug, or Default to the record.
Key field types must support owned cloning; the bound is diagnosed at the field.
Keys do not require Rust Eq, allowing checked floating-point key codecs;
identity uses canonical encoded physical values.
MapWriter/MapReader accept all Access modes and enforce path context. Their
finish methods validate key/index invariants and depth. The native Value
implementations own scalar conversion.

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
Public traits remain implementable; this is type discipline, not a security
boundary against deliberately dishonest trait implementations.

## 16. Test contract and CI

The skeleton workflow runs fmt, check, clippy with warnings denied, and tests
on the MSRV. Its current tests count is zero. It does not establish mapping,
SDK, emulator, or live-service correctness. Implementation PRs add these gates:

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
