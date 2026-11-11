# tabulary

**Status: early implementation.** Structured errors, paths, and encode/decode contexts
are implemented; record codecs and derives are still planned. The Rust below describes
the proposed API and does not yet compile against the library.

tabulary makes a Rust struct a typed DynamoDB item. A derive declares attribute
names and keys once. The same declaration drives strict decoding, expressions,
request preparation, and table definitions. Your application keeps its AWS client
and executes every request.

```rust,ignore
use tabulary::{ReadConsistency, Record, Table};

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
struct Order {
    #[tabulary(partition_key)]
    order_ref: String,
    #[tabulary(sort_key)]
    line_no: u64,
    quantity: u64,
}

async fn read_line(client: &aws_sdk_dynamodb::Client) -> Result<(), Box<dyn std::error::Error>> {
    let orders = Table::<Order>::new("orders")?;
    let request = orders
        .get(OrderKey::new("ord-amber".into(), 1)?, ReadConsistency::Strong)
        .build()?;
    let result = request.apply(client.get_item()).send().await;
    let read = request.decode(result)?;
    if let Some(order) = read.item {
        assert!(order.quantity > 0);
    }
    Ok(())
}
```

tabulary targets `aws-sdk-dynamodb` 1.x. It will support typed conditions and
updates, composite keys, sparse indexes, projections, explicit pagination,
batches, and transactions with typed cancellation reasons. It adds no retry loop
and makes no network calls. The SDK's own retry settings remain the caller's choice.

It is not a query planner, migration system, or opinionated single-table model.

- [API and implementation contract](docs/design.md)
- [Worked Rust examples](docs/examples/README.md)
- [Build milestones and acceptance tests](docs/milestones.md)

## Checking the workspace

Edition 2024; MSRV **Rust 1.97.1**. Both crates forbid unsafe code. The runtime
depends on `aws-sdk-dynamodb` with API floor **1.130.0** and default SDK features
disabled. Applications enable the SDK transport features they need. The default
`derive` feature includes the optional macro crate; `--no-default-features`
omits it. The macro implementations and runtime API are still future milestones.

CI runs formatting, check, clippy, and tests with the committed lockfile. A
separate MSRV matrix checks the exact SDK floor and the current compatible
release with both `--all-features` and `--no-default-features`. SDK fixtures
exercise attribute values and modeled conditional-failure items without I/O;
the dependency guard rejects SDK features and transitive transport dependencies.
The error and context acceptance tests run in the same workspace test step.

```sh
cargo check --workspace --all-targets --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python scripts/check_sdk_features.py --all-features
python scripts/check_sdk_features.py --no-default-features
```

Endpoint integration tests are specified in the design and milestones; the empty
skeleton does not yet run DynamoDB tests or type-check the proposed examples.

Licensed under [Apache-2.0](LICENSE).
