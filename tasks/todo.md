# Milestone 02

- [x] Read the design and milestone acceptance gates; verify the skeleton baseline.
- [x] Add external acceptance tests for paths, contexts, structured faults,
  redaction, and standard error/source behavior; observe failure first.
- [x] Implement and export SDK-independent error and context types in
  `crates/tabulary/src/errors.rs` without dependencies or lockfile changes.
- [x] Run fmt, check, clippy with warnings denied, and workspace tests; review diff.
- [ ] Push `m02-errors`, open one PR, and verify its CI without merging.

The design fixes decode faults but leaves encode/build variant details and context
constructors unspecified. Use structured metadata only, quoted attribute segments,
and child contexts that preserve their parent. Codec enforcement, schema handles,
validation hooks, SDK integration, and depth enforcement belong to later milestones.
