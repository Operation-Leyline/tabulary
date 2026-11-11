# tabulary: agent instructions

tabulary is a typed DynamoDB record library for Rust. This repository is public.

## Sources of truth

- `docs/design.md` is the API and implementation contract. Do not diverge from it silently: if it is ambiguous or wrong for the work in hand, say so in the pull request and change the document in the same pull request.
- `docs/milestones.md` is the build plan. Each row is one pull request with its acceptance tests and dependencies.
- `docs/examples/` are sketches of the proposed API. A milestone that makes one compile registers it as a real test.

## Rules

- One milestone per pull request, at the size the plan names. Do not start a milestone whose dependencies have not merged.
- Write the row's acceptance tests in the same pull request, failing first where practical. Never weaken, skip or ignore a test to get green, and never decide an outcome from error text.
- tabulary makes no network calls and retries nothing. It builds requests and decodes responses; the caller owns the client.
- No silent defaults: a missing, mistyped or malformed attribute is a typed error that names the record and the attribute.
- Nothing private or machine-specific goes in the repository: no names, schemas or text from any other codebase, no local paths, no workstation environment settings. Examples use neutral domains (orders, invoices, sessions).
- Do not commit task lists, notes or scratch files. Keep them outside the checkout.
- Leave `README.md` and `docs/milestones.md` alone unless the milestone's own row changes them: parallel pull requests that each edit a status line conflict with one another.
- In `crates/*/src/lib.rs`, keep one `mod` or `pub use` per line in alphabetical order, so parallel pull requests merge cleanly.
- Both crates forbid unsafe code. Dependencies are few and justified in the pull request that adds them.

## Checks

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python scripts/check_sdk_features.py --all-features
python scripts/check_sdk_features.py --no-default-features
```

CI runs these on the minimum supported Rust version and against both the SDK floor and the current SDK. A pull request is not done until CI is green and every review thread is answered.
