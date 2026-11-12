# Implementation milestones

Status: **design**. Only the workspace skeleton exists. The proposed API contract
is [design.md](design.md); the [examples](examples/README.md) are syntax-checked
source until the owning milestone makes them compile against the real crates.

Each row is one pull request, targeting **150–400 changed implementation and test
lines**, excluding generated diagnostic snapshots. Split a row further if it
exceeds that range. The exception is a piece that cannot be cut smaller and stay
coherent, such as a derive's parsing, validation and expansion with its
round-trip tests: it may run to about 700 lines, and its description states the
size and why it was not split. Avoid bundling adjacent rows merely because they touch the
same file. No milestone implements application schemas or business policies.

Dependencies name earlier PRs, so the table is in dependency order. “Parallel”
means the work may start after its dependencies land, alongside other ready
rows; it never means bypassing a dependency or landing an unbuildable commit.
Keep each feature's public API narrow until its tests land.

## Build sequence and acceptance gates

| PR | Scope | Depends on | Acceptance tests | Parallel work |
| --- | --- | --- | --- | --- |
| 01 | Add SDK dependency with default features disabled, API floor 1.130.0, and maintain the lockfile. | Skeleton | MSRV check on exact SDK floor and current compatible SDK, no transport dependency enabled by tabulary, all-features/no-default-features check. | With 02 and macro parsing setup. |
| 02 | Structured decode/encode/build errors, contexts, and path segments. | Skeleton | Escaped literal-dot attributes, nested list paths, custom structured code, redacted Debug/Display, error/source behavior. | With 01. |
| 03 | String, bool, binary Vec and fixed-array Value codecs. | 01, 02 | Exact AV tags, empty non-key values, wrong type, array length, unknown future SDK type fault. | With 04, 07, 08. |
| 04 | Canonical decimal parser and exact u64/i64/u128 native number codecs. | 01, 02 | Integer bounds, integral exponent forms, fractional rejection, decimal precision/range, zero normalization, u128::MAX rejection without rounding. | With 03, 07, 08. |
| 05 | Checked f64 codec and decimal equivalence test. | 04 | Shortest decimal round trip, inexact read rejection, NaN/infinities, negative zero, underflow/overflow, accepted service boundary values. | With 06 after 04. |
| 06 | List, dynamic map, string/number/binary set codecs. | 03, 04 | Empty list/map retained, nonempty sets, normalized duplicates rejected, deterministic encoding, nested path indexes. | With 05. |
| 07 | Optional field presence, Nullable, Converter trait, and u128_string converter. | 01, 02 | Missing versus NULL versus wrong type, NULL(false) rejection, full u128 range through S, constants use converter, no implicit defaults. | With 03, 04. |
| 08 | TTL/time unit wrappers and Secret. | 03, 04 | Unit conversion overflow/precision loss, pre-epoch signed timestamps, exact Debug output is empty for Secret, raw value only via explicit accessor. | With expression work once ready. |
| 09 | BilledSize estimates and conservative model. | 05, 06, 07 | UTF-8, raw binary, map/list/set overhead, top-level rules, negative numbers, checked arithmetic and conservative max check. Label remote billing equivalence unverified. | With schema work. |
| 10 | Static schema descriptors and typed Attr presence/access/codec markers. | 03, 04, 07 | Manual neutral record descriptors, foreign owner rejection, immutable key capability checks, forged descriptor build rejection. | With 09. |
| 11 | Record derive for named field keys, renames, explicit defaults, and unknown policy. | 10 | Expansion fixture, record/key round trips, required missing/wrong type, default runs only on missing, duplicate-name spans, optional/non-scalar primary-key compile failures. | With 19 after 10. |
| 12 | Document derive, nested record Value expansion, generic bounds, and depth checks. | 06, 11 | Nested error paths, no Vec<u8>/list ambiguity, generic records, renamed dependency path, borrowed-record rejection, maximum nesting boundary. | With 13, 14, and 19. |
| 13 | StringEnum derive. | 03, 11 | Variant rename rules, unknown tags, duplicate values, unit-only compile errors. | With 12 and 14. |
| 14 | Composite template parser and compile-time ambiguity checks. | 11 | Literal braces, unknown fields, duplicate components, adjacent variable components, invalid codecs/separators; diagnostics point at template or field. | With 12 and 13. |
| 15 | Template codec formatting and inverse validation in generated keys and record decode. | 03, 04, 14 | UTF-8 hex, fixed bytes, dec/dec20, wrong prefix/length, source mismatch, primary source immutability, complete-component query prefix fixture. | With 18 once dependencies ready. |
| 16 | GSI/LSI metadata and typed index handles. | 15 | Hash-only GSI, local partition equality, physical type/name collision checks, projected include names from fields, non-key AttributeDefinitions excluded. | With 18 and expressions. |
| 17 | Sparse index encode/decode and synthesized-key invariants. | 07, 16 | Missing membership input omits synthetic keys, partial reused fields allowed, malformed present input rejected, partial synthetic keys and source mismatch rejected. | With 18 and expressions. |
| 18 | Variants derive and flattened/nested strict codec. | 12, 13 | Each shape round trips; missing/wrong/unknown discriminator; inactive payload rejected under allow-unknown; shared field compatibility, multiple-flatten compile failure. | With 15–17 where ready. |
| 19 | Condition AST and logical/typed operand capabilities. | 10 | Owner/type/codec compile failures, exists/not-exists, comparisons, begins_with, size, contains, bounded IN, explicit boolean grouping. | With 11–18. |
| 20 | Condition renderer and shared alias allocator. | 19 | Exact text/maps for reserved names, literal dots, deterministic deduped names, nondeduped values, no unused bindings, secret values redacted in Debug. | With 21. |
| 21 | Update actions, SET operands, and protected target rules. | 19 | set/remove/add/delete-members/increment, required removal compile failure, if_not_exists and append_or_empty are explicit, key/version/source protections. | With 20. |
| 22 | Nested update paths and canonical action rendering. | 12, 20, 21 | SET/REMOVE/ADD/DELETE order, duplicate/ancestor overlap rejection, list indexes, dynamic map key aliasing, parent map not implicitly created. | With 23 and 24. |
| 23 | Primary/index key-condition builders. | 16, 20 | Partition equality required, single sort predicate, no OR/non-key operands, begins_with wire constraints, composite prefix boundary checks. | With 22 and 24. |
| 24 | Projection derive and selected-attribute decode. | 12, 18 | Owner/type/presence/codec checks, inherited explicit defaults, omitted required base fields allowed outside view, nested view, whole variant group, non-ALL index compatibility. | With 22 and 23. |
| 25 | GetItem prepared adapter and read output/error types. | 01, 24 | Full SDK input apply equality including clearing absent fields, explicit consistency compile gate, absent versus empty item, strict decode, consumed capacity retained on decode fault. | With cursor/table-definition work. |
| 26 | Single-write refusal classifier and OldItem decoding. | 02, 25 | Modeled condition/conflict/throttle variants, false-positive message strings remain SDK errors, missing/malformed old item, redacted source access. | With cursor/table-definition work. |
| 27 | PutItem prepared adapter. | 09, 17, 26 | Conditional put, ALL_OLD failure option, inspectable maps, no I/O on build/apply/decode, return and capacity fields. Promote 01_simple.rs to compile test. | With 28 after 26. |
| 28 | DeleteItem prepared adapter. | 26 | Key-only request, inline condition, ALL_OLD success and failure decode, missing old item, exact input field copies. | With 27. |
| 29 | UpdateItem prepared adapter. | 22, 26 | Atomic mutation plus condition, old/new full returns, partial-return modes rejected, wrong response shape, consumed capacity and collection metrics. | With 27 and 28. |
| 30 | Version guards on all single/transaction-ready write builders. | 27, 28, 29 | Missing policy fails, expect-new explicit version 1, stale/absent version rejection, checked increment overflow, bypass visible, AND user conditions, no record mutation. Compile 04_conditional_update.rs. | With 31 and 32 after 29. |
| 31 | Transition updates for flattened variants. | 18, 29 | Guard expected tag, SET active payload, REMOVE all inactive-only fields, overlap detection with common changes, old-shape failure decode. Compile 03_status_record.rs. | With 30 and 32. |
| 32 | Explicit reindex update expansion. | 17, 29 | Full source tuple, synthetic key updates/removal, shared dependency agreement, incomplete index set failure, primary-source mutation rejection. | With 30 and 31. |
| 33 | Cursor object and complete canonical request binding. | 23, 24 | Full service key retention, empty end marker, schema/table/index/filter/direction/projection/consistency/segment mismatch, per-page limit may change. | With 25–32 after dependencies. |
| 34 | Query prepared adapter and one-page decode. | 23, 25, 33 | Empty filtered page with continuation, key-filter misuse, exact limit/order behavior, GSI strong rejection, no automatic requests, strict corrupt-page metadata. | With 35 and 37. |
| 35 | Scan prepared adapter and segment validation. | 25, 33 | Unsegmented and parallel scan inputs, invalid segment/total, segment-bound resumption, consistency required, no fan-out. | With 34 and 37. |
| 36 | Versioned cursor bytes and bounded parser. | 33, 34, 35 | All key scalar types, deterministic canonical maps, byte round trip, bounds before allocation, truncated/trailing/unknown-version input, filter constants retained privately. | With 38 when ready. |
| 37 | Table definition SDK inputs, merge validation, and TTL input. | 08, 17 | Primary/GSI/LSI/projection fixtures, explicit billing/throughput, incompatible merged schemas, absent versus present separate TTL request; never invoke client. | With 34–36. |
| 38 | Endpoint harness and actual emulator CI job. | 25, 27, 28, 29, 37 | Explicit endpoint env required; pinned emulator starts with deadline; create/activate/use/delete isolated table; CRUD and condition-old-item integration. Absence is unexecuted, not passed. | With transaction and batch work. |
| 39 | Transaction action conversion, condition checks, and request builder. | 26, 27, 28, 29 | Mixed put/update/delete/check inputs, action-local maps, common capacity and caller token, duplicate table/key rejection, no auto split. | With 42 and 44. |
| 40 | Typed transaction tuple decoding. | 39 | Three heterogeneous reasons and old records, all known machine codes, null/None code, unknown code, absent and wrong-length vectors, malformed old items do not replace refusal. Compile 05_transaction.rs. | With batch adapters. |
| 41 | Nested tuple/vector transaction composition. | 40 | Depth-first order, mixed vector and tuple shape, empty vector contribution, final nonempty cap, 100 and over-cap actions, compile-time record-type alignment. | With 43 and 45. |
| 42 | BatchGet progress state and fixed call budget. | 25 | Nonzero budget including first call, one outstanding round, wrong/replayed chunk, ordered found/missing/pending states, duplicate keys, no implicit chunking. | With 39 and 44. |
| 43 | BatchGet SDK adapter and projection correlation. | 24, 42 | Reordered responses, auto-added primary keys, unexpected keys, unprocessed subset, decode/SDK errors preserve progress, finish reports exhausted budget. Compile 06_query_batch.rs after 34. | With 40–41 and 45. |
| 44 | BatchWrite progress state. | 27, 28 | Put/delete identities, duplicate target, versioned record/condition/update rejection, one-round rules, indeterminate failed writes are not retried. | With 39 and 42. |
| 45 | BatchWrite SDK adapter and partial errors. | 44 | Unprocessed subset only, request/action caps, exact capacity vectors by round, budget exhaustion contains all processed and pending actions, abort-round behavior. | With 40–43. |
| 46 | Complete request-size and operation-limit checks. | 09, 22, 41, 43, 45 | Item versus wire/base64 size, expression/path limits, batch limits, known versus unknown transaction size, checked arithmetic. No claim that current rows' size is knowable. | With endpoint suites. |
| 47 | Indexed, variant, projection, version, and pagination endpoint coverage. | 30, 31, 32, 34, 35, 36, 38 | Sparse membership changes, ALL/non-ALL views, query/cursor pages, scan segments, simultaneous stale versions, transition cleanup, stored TTL value. Compile 02_composite_sparse.rs and 07_values_projection_scan.rs. | With 48. |
| 48 | Transaction and batch endpoint coverage. | 38, 41, 43, 45 | Atomic three-item success/failure, typed returned old items, failed same-target transaction rejected locally, batch result order, explicit multiple sends. Deterministic fixtures cover uninducible throttles and partial batches. | With 47. |
| 49 | Finish compile-fail matrix, example registration, and public contract review. | 46, 47, 48 | All seven real example modules compile on SDK floor/MSRV; all invalid examples fail at intended spans; all required unit/emulator gates run; default/feature builds agree. Update design-status text only to reflect actual implemented evidence. | Final join. |

## Test execution

Every PR runs the existing workspace fmt, check, clippy, and test workflow.
Add focused acceptance tests in the same PR as behavior. A failing test must
prove the missing behavior before its implementation where practical; snapshots
alone do not prove semantic correctness.

PR 38 adds a separate endpoint job. In that job:

1. Start dynoxide at a pinned revision on a loopback port.
2. Set `TABULARY_DYNAMODB_ENDPOINT`, a region, and dummy AWS credentials.
3. Perform bounded readiness checks.
4. Run `cargo test -p tabulary --test endpoint -- --ignored`.
5. Preserve logs and report endpoint capability differences explicitly.

Without the endpoint variable, a deliberate ignored-test invocation fails. A
normal `cargo test` leaves those tests unexecuted. A real-service run uses the
same endpoint variable and tests with real credentials and isolated temporary
tables. Keep its results distinct from emulator results.

Do not retry failures into success, ignore unprocessed keys, weaken strict decode,
or use error text to satisfy an acceptance gate. SDK call-count tests configure
SDK retries explicitly; tabulary itself must always issue zero calls.

## Release experiments

The [open questions](design.md#18-open-questions-and-recommendations) have fixed
initial recommendations. SDK floor verification belongs to PR 01. Numeric size
boundary experiments require real-service evidence before exact billing claims.
Large tuple ergonomics can be measured after PR 41; cursor cross-major stability
and token signing are not first-version requirements.

Passing the current skeleton proves only workspace integrity. Completing the
sequence requires all seven examples to compile against the actual API and
the endpoint test suite to run in CI.
