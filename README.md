# tabulary

**Status: design.** The workspace builds, but its two crates contain no API yet.
The Rust below describes the proposed API; it does not compile against this skeleton.

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

## Checking the skeleton

Edition 2024; MSRV **Rust 1.97.1**. Both crates forbid unsafe code. There are no
third-party dependencies in the skeleton. CI runs formatting, check, clippy, and
tests; there are no behavior tests until implementation begins.

On Windows, keep build output outside the checkout:

```powershell
$env:AWS_LC_SYS_PREBUILT_NASM = "1"
$env:CARGO_TARGET_DIR = "D:/lwt/tabulary-target"
cargo check --workspace --all-targets --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

On other platforms, set `CARGO_TARGET_DIR` to an external temporary directory.
Endpoint integration tests are specified in the design and milestones; the empty
skeleton does not yet run DynamoDB tests or type-check the proposed examples.

Licensed under [Apache-2.0](LICENSE).
