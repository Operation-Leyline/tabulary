# Worked API examples

These seven modules compile against the real library in integration tests.
The registrations below are under `crates/tabulary/tests/`. In addition,
`examples.rs` imports every module and calls every public function with assertions
on its results. Async examples use deterministic SDK responses through an in-memory
connector; those tests perform no network I/O. The endpoint suite tests the same
library features against the pinned emulator using isolated table names.
CI runs the modules on the MSRV with the SDK floor and current compatible SDK.
They require the `derive` feature; runtime tests also run without that feature.

| File | Contract exercised | Test registration |
| --- | --- | --- |
| [01_simple.rs](01_simple.rs) | Declared names, field keys, strict round trip, explicit strong GetItem, inspectable conditional put. | `put_item.rs`; `examples.rs` |
| [02_composite_sparse.rs](02_composite_sparse.rs) | Reversible hex key, optional index membership, typed GSI query, table definition. | `endpoint/indexed.rs`; `examples.rs` |
| [03_status_record.rs](03_status_record.rs) | Discriminator-selected row shape and atomic cleanup when changing variants. | `variant_transitions.rs`; `examples.rs` |
| [04_conditional_update.rs](04_conditional_update.rs) | Inline guard, numeric update, version expectation, typed conditional failure with old record. | `version_guards.rs`; `examples.rs` |
| [05_transaction.rs](05_transaction.rs) | Three record types in three tables, put/update/check, typed cancellation tuple. | `transaction_decode.rs`; `examples.rs` |
| [06_query_batch.rs](06_query_batch.rs) | Bounded caller pagination and explicit bounded unprocessed-key submissions. | `batch_get_adapter.rs`; `examples.rs` |
| [07_values_projection_scan.rs](07_values_projection_scan.rs) | Value types, nested document paths, explicit defaults, projection, and parallel scan. | `field_converters.rs`, `endpoint/indexed.rs`; `examples.rs` |

Rustfmt checks the example source. None of the seven example files was changed
for the final registration review. SDK retries remain the caller's choice;
`examples.rs` explicitly disables them when checking call order and count.
Real-service boundary checks remain unexecuted as listed in
[the test contract](../design.md#16-test-contract-and-ci).
