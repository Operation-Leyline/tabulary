# Design work

- [x] Inspect the empty repository and public reference APIs.
- [x] Read authorized requirements sources without copying their identifiers.
- [x] Specify mapping, schemas, expressions, requests, errors, and test gates.
- [x] Write neutral Rust examples and dependency-ordered milestones.
- [x] Add the two-crate skeleton and portable CI.
- [x] Check consistency, formatting, clippy, tests, and identifier hygiene.
- [x] Commit locally on design; do not push.

Risk: the proposed API is a contract, while the crates remain empty. Examples must
be marked as proposals and promoted to compile checks during implementation.
All local Cargo output belongs in D:/lwt/tabulary-target.

Verified locally on Rust 1.97.1: workspace check, fmt, clippy with warnings denied,
and tests (zero behavior tests in the skeleton). All seven proposed sources pass
rustfmt parsing/formatting. Local links, Markdown fences, workspace manifests,
CI YAML, and milestone dependency order were checked. The private-identifier
scan returned no matches. No API examples or DynamoDB behavior were type-checked
or executed against an implementation.
