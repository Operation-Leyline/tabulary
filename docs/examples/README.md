# Worked API proposals

These are self-contained Rust source sketches for the proposed API in
[the design](../design.md). The crates are empty, so these files are not currently
Cargo targets and have not been type-checked against tabulary. They contain
ordinary functions that accept the caller's client; no mock API is supplied.
Rustfmt checks their syntax and formatting.

| File | Contract exercised |
| --- | --- |
| [01_simple.rs](01_simple.rs) | Declared names, field keys, strict round trip, explicit strong GetItem, inspectable conditional put. |
| [02_composite_sparse.rs](02_composite_sparse.rs) | Reversible hex key, optional index membership, typed GSI query, table definition. |
| [03_status_record.rs](03_status_record.rs) | Discriminator-selected row shape and atomic cleanup when changing variants. |
| [04_conditional_update.rs](04_conditional_update.rs) | Inline guard, numeric update, version expectation, typed conditional failure with old record. |
| [05_transaction.rs](05_transaction.rs) | Three record types in three tables, put/update/check, typed cancellation tuple. |
| [06_query_batch.rs](06_query_batch.rs) | Bounded caller pagination and explicit bounded unprocessed-key submissions. |
| [07_values_projection_scan.rs](07_values_projection_scan.rs) | Value types, nested document paths, explicit defaults, projection, and parallel scan. |

Milestones promote each file into a compile test against the real implementation.
Until then, a green skeleton check says nothing about these proposed APIs.
Native SDK retries are controlled through the supplied client. Every visible
send invocation can therefore include multiple SDK attempts unless the caller
configures the client otherwise.
