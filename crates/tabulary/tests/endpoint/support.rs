//! Network execution belongs only to tests. No implicit endpoint or retries.
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aws_sdk_dynamodb::{
    Client,
    config::{retry::RetryConfig, timeout::TimeoutConfig},
    types::TableStatus,
};
use tabulary::{Billing, Record};
use tokio::time::{sleep, timeout};

const WAIT: Duration = Duration::from_secs(60);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq)]
enum EndpointError {
    Missing,
    InvalidUrl,
}

fn endpoint(value: Option<&str>) -> Result<url::Url, EndpointError> {
    let value = value.ok_or(EndpointError::Missing)?;
    let url = url::Url::parse(value).map_err(|_| EndpointError::InvalidUrl)?;
    if !(value.starts_with("http://") || value.starts_with("https://"))
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(EndpointError::InvalidUrl);
    }
    Ok(url)
}

async fn client() -> Client {
    // Check before loading credentials/configuration or making any SDK call.
    let value = std::env::var("TABULARY_DYNAMODB_ENDPOINT");
    let endpoint = endpoint(value.as_deref().ok()).unwrap_or_else(|error| {
        eprintln!("TABULARY_DYNAMODB_ENDPOINT requires an explicit HTTP(S) URL: {error:?}; endpoint suite UNEXECUTED");
        std::panic::panic_any(error)
    });
    let config = timeout(
        WAIT,
        aws_config::defaults(aws_config::BehaviorVersion::latest()).load(),
    )
    .await
    .expect("AWS configuration deadline exceeded");
    assert!(
        config.region().is_some(),
        "configure an AWS region explicitly"
    );
    Client::from_conf(
        aws_sdk_dynamodb::config::Builder::from(&config)
            .endpoint_url(endpoint.as_str())
            .retry_config(RetryConfig::disabled())
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(3))
                    .operation_timeout(Duration::from_secs(10))
                    .build(),
            )
            .build(),
    )
}

fn table_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "tabulary_endpoint_{}_{nanos}_{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

async fn wait_table(client: &Client, name: &str, deleted: bool) {
    timeout(WAIT, async {
        loop {
            match client.describe_table().table_name(name).send().await {
                Ok(output) => {
                    if !deleted
                        && output.table().and_then(|t| t.table_status())
                            == Some(&TableStatus::Active)
                    {
                        return;
                    }
                }
                Err(error)
                    if error
                        .as_service_error()
                        .is_some_and(|e| e.is_resource_not_found_exception()) =>
                {
                    if deleted {
                        return;
                    }
                }
                Err(error) => panic!("DescribeTable for {name}: {error:?}"),
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("table {name} deadline exceeded (deleted={deleted})"));
}

/// Each test owns one table. Cleanup runs even when activation or assertions panic.
pub async fn with_table<R, F, Fut>(body: F)
where
    R: Record,
    F: FnOnce(Client, String) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let client = client().await;
    let name = table_name();
    let input = R::table_definition()
        .create_input(&name, Billing::OnDemand)
        .unwrap();
    client
        .create_table()
        .set_table_name(input.table_name)
        .set_attribute_definitions(input.attribute_definitions)
        .set_key_schema(input.key_schema)
        .set_billing_mode(input.billing_mode)
        .set_provisioned_throughput(input.provisioned_throughput)
        .set_global_secondary_indexes(input.global_secondary_indexes)
        .set_local_secondary_indexes(input.local_secondary_indexes)
        .send()
        .await
        .unwrap_or_else(|e| panic!("CreateTable for {name}: {e:?}"));
    eprintln!("created {name}");
    let test_client = client.clone();
    let test_name = name.clone();
    let result = tokio::spawn(async move {
        wait_table(&test_client, &test_name, false).await;
        body(test_client, test_name).await;
    })
    .await;
    // Never enumerate tables or clean up a table whose creation was not successful.
    client
        .delete_table()
        .table_name(&name)
        .send()
        .await
        .unwrap_or_else(|e| panic!("DeleteTable for {name}: {e:?}; test result: {result:?}"));
    wait_table(&client, &name, true).await;
    eprintln!("deleted {name}");
    if let Err(error) = result {
        std::panic::resume_unwind(error.into_panic());
    }
}

#[test]
fn endpoint_is_explicit_and_never_defaults() {
    for value in [
        None,
        Some(""),
        Some("127.0.0.1:8000"),
        Some("http:localhost"),
        Some("ftp://localhost"),
        Some("http://user:secret@localhost"),
        Some("http://localhost/#fragment"),
    ] {
        assert!(endpoint(value).is_err());
    }
    for value in [
        "http://127.0.0.1:8000",
        "https://dynamodb.us-east-1.amazonaws.com",
    ] {
        assert!(endpoint(Some(value)).is_ok());
    }
}

#[test]
fn table_names_are_isolated_and_service_compatible() {
    let names: std::collections::HashSet<_> = (0..1000).map(|_| table_name()).collect();
    assert_eq!(names.len(), 1000);
    assert!(
        names
            .iter()
            .all(|n| n.len() <= 255 && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    );
}

#[test]
fn missing_endpoint_fails_before_aws_configuration() {
    const CHILD: &str = "TABULARY_ENDPOINT_PRECONDITION_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let panic = tokio::spawn(client()).await.unwrap_err().into_panic();
            assert_eq!(
                *panic.downcast::<EndpointError>().unwrap(),
                EndpointError::Missing
            );
        });
    } else {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "support::missing_endpoint_fails_before_aws_configuration",
            ])
            .env_remove("TABULARY_DYNAMODB_ENDPOINT")
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "isolated precondition check: {result:?}"
        );
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .lines()
                .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;"))
        );
    }
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn panicking_test_body_still_deletes_its_table() {
    let name = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured = name.clone();
    let result = tokio::spawn(with_table::<super::Order, _, _>(|_, table| async move {
        *captured.lock().unwrap() = Some(table);
        panic!("intentional body failure to verify cleanup");
    }))
    .await;
    assert!(result.unwrap_err().is_panic());
    let table = name.lock().unwrap().clone().expect("body must have run");
    let error = client()
        .await
        .describe_table()
        .table_name(table)
        .send()
        .await
        .unwrap_err();
    assert!(
        error
            .as_service_error()
            .is_some_and(|e| e.is_resource_not_found_exception())
    );
}
