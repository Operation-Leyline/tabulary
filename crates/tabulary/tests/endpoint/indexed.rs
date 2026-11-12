//! Milestone 47: indexed reads, guarded mutations, and caller-owned pagination.
use std::{collections::BTreeSet, num::NonZeroU32, time::Duration};

use aws_sdk_dynamodb::{Client, operation::query::QueryOutput, types::AttributeValue as Av};
use tabulary::{
    BuildError, Cursor, Decimal, DecodeFault, FailureKind, OldItem, OldItemMode, QueryOrder,
    ReadConsistency::{Eventual, Strong},
    ReadError, Record, ReturnItem, ScanSegment, Table, TtlSeconds, Update, WriteError,
};
use tokio::time::{sleep, timeout};

#[allow(dead_code)]
#[path = "../../../../docs/examples/02_composite_sparse.rs"]
mod composite_example;

fn limit(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}

#[test]
fn composite_design_example_compiles_and_maps_real_records() {
    composite_example::mapping_and_definition().unwrap();
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case",
    index(ident = ByRoute, name = "by_route", kind = global, sparse,
        partition(name = "gpk", template = "route!{route:hex}"), sort = line, project = all),
    index(ident = ByPosition, name = "by_position", kind = local,
        partition = id, sort = position, project = keys_only))]
struct Parcel {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(sort_key)]
    line: u64,
    route: Option<String>,
    position: u64,
    note: String,
}

#[derive(Debug, PartialEq, tabulary::Projection)]
#[tabulary(record = Parcel)]
struct Position {
    #[tabulary(attribute = ParcelAttrs::POSITION)]
    position: u64,
}

async fn route_members(client: &Client, table: &Table<Parcel>, route: &str, expected: &[u64]) {
    // Poll only for eventual index propagation, never retry a failed request.
    timeout(Duration::from_secs(30), async {
        loop {
            let request = table
                .index(ParcelIndexes::BY_ROUTE)
                .query(ParcelIndexes::BY_ROUTE.partition(route.into()), Eventual)
                .build()
                .unwrap();
            let page = request
                .decode(request.apply(client.query()).send().await)
                .unwrap();
            if page.items.iter().map(|row| row.line).collect::<Vec<_>>() == expected {
                assert!(page.next.is_none());
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("sparse GSI propagation deadline");
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn sparse_reindex_and_all_keys_only_views() {
    super::support::with_table::<Parcel, _, _>(|client, name| async move {
        let table = Table::<Parcel>::new(name).unwrap();
        let record = Parcel {
            id: "parcel".into(),
            line: 1,
            route: None,
            position: 7,
            note: "ready".into(),
        };
        let put = table.put(&record).build().unwrap();
        put.decode(put.apply(client.put_item()).send().await)
            .unwrap();
        route_members(&client, &table, "lime", &[]).await;
        for route in [Some("lime"), Some("cyan"), None] {
            let update = table
                .update(record.key().unwrap())
                .changes(Update::new().reindex(
                    ParcelIndexes::BY_ROUTE,
                    ParcelByRouteInputs::new(route.map(str::to_owned), 1),
                ))
                .return_item(ReturnItem::AllNew)
                .build()
                .unwrap();
            let result = update
                .decode(update.apply(client.update_item()).send().await)
                .unwrap();
            assert_eq!(result.item.unwrap().route.as_deref(), route);
            for name in ["lime", "cyan"] {
                route_members(
                    &client,
                    &table,
                    name,
                    if route == Some(name) { &[1] } else { &[] },
                )
                .await;
            }
        }
        let get = table.get(record.key().unwrap(), Strong).build().unwrap();
        let raw = get
            .apply(client.get_item())
            .send()
            .await
            .unwrap()
            .item
            .unwrap();
        assert!(!raw.contains_key("route"));
        assert!(!raw.contains_key("gpk"));
        assert_eq!(Parcel::from_item(&raw).unwrap().position, 7);

        for line in 2..=3 {
            let row = Parcel {
                id: "parcel".into(),
                line,
                route: Some("lime".into()),
                position: 7,
                note: "ready".into(),
            };
            let put = table.put(&row).build().unwrap();
            put.decode(put.apply(client.put_item()).send().await)
                .unwrap();
        }
        route_members(&client, &table, "lime", &[2, 3]).await;
        let indexed = || {
            table
                .index(ParcelIndexes::BY_ROUTE)
                .query(ParcelIndexes::BY_ROUTE.partition("lime".into()), Eventual)
                .select::<Position>()
        };
        let request = indexed().limit(limit(1)).build().unwrap();
        let page = request
            .decode(request.apply(client.query()).send().await)
            .unwrap();
        assert_eq!(page.items, [Position { position: 7 }]);
        let cursor = page.next.unwrap();
        assert_eq!(cursor.last_evaluated_key().len(), 3);
        assert_eq!(cursor.last_evaluated_key()["line"], Av::N("2".into()));
        let bytes = cursor.to_bytes().unwrap();
        let request = indexed()
            .limit(limit(3))
            .resume(Cursor::from_bytes(&bytes).unwrap())
            .build()
            .unwrap();
        let page = request
            .decode(request.apply(client.query()).send().await)
            .unwrap();
        assert_eq!(page.items, [Position { position: 7 }]);
        assert!(page.next.is_none());

        assert!(matches!(
            table
                .index(ParcelIndexes::BY_ROUTE)
                .query(ParcelIndexes::BY_ROUTE.partition("lime".into()), Strong)
                .build(),
            Err(BuildError::UnsupportedConsistency)
        ));
        let local = table.index(ParcelIndexes::BY_POSITION);
        assert!(
            local
                .query(
                    ParcelIndexes::BY_POSITION.partition("parcel".into()),
                    Strong
                )
                .build()
                .is_err()
        );
        let request = local
            .query(
                ParcelIndexes::BY_POSITION.partition("parcel".into()),
                Strong,
            )
            .select::<Position>()
            .build()
            .unwrap();
        assert_eq!(
            request
                .decode(request.apply(client.query()).send().await)
                .unwrap()
                .items,
            [
                Position { position: 7 },
                Position { position: 7 },
                Position { position: 7 }
            ]
        );
    })
    .await;
}

#[derive(Debug, PartialEq, tabulary::Variants)]
#[tabulary(tag = "stage", rename_all = "snake_case")]
enum Stage {
    Waiting {
        queued: u64,
        note: Option<String>,
    },
    Sent {
        tracking: String,
        note: Option<String>,
    },
    Done,
}

#[derive(Debug, PartialEq, tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Delivery {
    #[tabulary(partition_key)]
    id: String,
    #[tabulary(version)]
    revision: u64,
    #[tabulary(flatten)]
    stage: Stage,
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn simultaneous_stale_versions_and_transition_cleanup() {
    super::support::with_table::<Delivery, _, _>(|client, name| async move {
        let table = Table::<Delivery>::new(name).unwrap();
        let original = Delivery {
            id: "delivery".into(),
            revision: 1,
            stage: Stage::Waiting {
                queued: 4,
                note: Some("gift".into()),
            },
        };
        let put = table.put(&original).expect_new().build().unwrap();
        put.decode(put.apply(client.put_item()).send().await)
            .unwrap();
        let request = || {
            table
                .update(original.key().unwrap())
                .expect_version(1)
                .on_condition_failure(OldItemMode::All)
                .return_item(ReturnItem::AllNew)
                .build()
                .unwrap()
        };
        let (a, b) = (request(), request());
        let (a_raw, b_raw) = tokio::join!(
            a.apply(client.update_item()).send(),
            b.apply(client.update_item()).send()
        );
        let (winner, loser) = match (a.decode(a_raw), b.decode(b_raw)) {
            (Ok(winner), Err(loser)) | (Err(loser), Ok(winner)) => (winner.item.unwrap(), loser),
            other => panic!("exactly one guarded write must commit: {other:?}"),
        };
        assert_eq!(winner.revision, 2);
        let WriteError::Refused { failure, source } = loser else {
            panic!("modeled stale refusal required")
        };
        assert!(
            source
                .as_service_error()
                .is_some_and(|e| e.is_conditional_check_failed_exception())
        );
        assert_eq!(failure.kind, FailureKind::ConditionFailed);
        assert!(matches!(failure.old, OldItem::Decoded(old) if old == winner));
        let get = table.get(original.key().unwrap(), Strong).build().unwrap();
        assert_eq!(
            get.decode(get.apply(client.get_item()).send().await)
                .unwrap()
                .item,
            Some(winner)
        );
        let sent = table
            .update(original.key().unwrap())
            .expect_version(2)
            .changes(Update::new().transition(
                DeliveryAttrs::STAGE,
                Stage::Sent {
                    tracking: "parcel-cyan".into(),
                    note: None,
                },
                StageTags::WAITING,
            ))
            .return_item(ReturnItem::AllNew)
            .build()
            .unwrap();
        let sent = sent
            .decode(sent.apply(client.update_item()).send().await)
            .unwrap()
            .item
            .unwrap();
        assert_eq!(sent.revision, 3);
        let raw = get.apply(client.get_item()).send().await.unwrap();
        let item = raw.item().unwrap();
        assert!(!item.contains_key("queued"));
        assert!(!item.contains_key("note"));
        assert_eq!(get.decode(Ok(raw)).unwrap().item, Some(sent));
        let done = table
            .update(original.key().unwrap())
            .expect_version(3)
            .changes(Update::new().transition(DeliveryAttrs::STAGE, Stage::Done, StageTags::SENT))
            .return_item(ReturnItem::AllNew)
            .build()
            .unwrap();
        assert_eq!(
            done.decode(done.apply(client.update_item()).send().await)
                .unwrap()
                .item
                .unwrap()
                .revision,
            4
        );
        let raw = get.apply(client.get_item()).send().await.unwrap();
        for stale in ["queued", "note", "tracking"] {
            assert!(!raw.item().unwrap().contains_key(stale));
        }
        assert_eq!(
            get.decode(Ok(raw)).unwrap().item.unwrap().stage,
            Stage::Done
        );
    })
    .await;
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Reading {
    #[tabulary(partition_key)]
    group: String,
    #[tabulary(sort_key)]
    sequence: Decimal,
    visible: bool,
}

// Dynoxide expands extreme N keys beyond the cursor's documented 45-byte cap.
// Assert the refusal without changing library behavior, then explicitly substitute
// a compact-response fixture to exercise byte resume against the same endpoint.
fn numeric_page(
    request: &tabulary::PreparedQuery<Reading, ReadingPrimary>,
    mut raw: QueryOutput,
) -> tabulary::Page<Reading, Cursor<Reading, ReadingPrimary>> {
    if let Some(key) = raw.last_evaluated_key.as_ref()
        && key["sequence"].as_n().unwrap().len() > 45
    {
        let original = key.clone();
        let number = original["sequence"].as_n().unwrap();
        eprintln!(
            "endpoint limitation: expanded N is {} bytes; compact fixture follows",
            number.len()
        );
        let Err(ReadError::Decode(error)) = request.decode(Ok(raw.clone())) else {
            panic!("overlong numeric continuation must be refused")
        };
        assert_eq!(
            error.source.kind,
            DecodeFault::Custom {
                code: "invalid_page_cursor"
            }
        );
        assert_eq!(error.source.attribute(), Some("LastEvaluatedKey"));
        assert_eq!(error.page.unwrap().last_evaluated_key(), Some(&original));
        let compact = number.parse::<Decimal>().unwrap().to_string();
        raw.last_evaluated_key
            .as_mut()
            .unwrap()
            .insert("sequence".into(), Av::N(compact));
    }
    request.decode(Ok(raw)).unwrap()
}

#[test]
fn compact_extreme_cursor_fixture_round_trips_original_key_bytes() {
    let table = Table::<Reading>::new("readings_fixture").unwrap();
    let make = || table.query(Reading::primary().partition("readings".into()), Strong);
    let request = make().build().unwrap();
    for number in [
        "-99999999999999999999999999999999999999e88",
        "-1e-130",
        "1e-130",
        "99999999999999999999999999999999999999e88",
    ] {
        let key = tabulary::Item::from([
            ("group".into(), Av::S("readings".into())),
            ("sequence".into(), Av::N(number.into())),
        ]);
        let raw = QueryOutput::builder()
            .count(0)
            .scanned_count(1)
            .set_last_evaluated_key(Some(key.clone()))
            .build();
        let cursor = request.decode(Ok(raw)).unwrap().next.unwrap();
        let bytes = cursor.to_bytes().unwrap();
        let resumed = make()
            .resume(Cursor::from_bytes(&bytes).unwrap())
            .build()
            .unwrap();
        assert_eq!(resumed.input().exclusive_start_key(), Some(&key));
    }
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn extreme_number_query_pages_resume_from_bytes_and_keep_empty_filtered_pages() {
    super::support::with_table::<Reading, _, _>(|client, name| async move {
        let table = Table::<Reading>::new(name).unwrap();
        let values: Vec<Decimal> = [
            "-9.9999999999999999999999999999999999999e125",
            "-1e-130",
            "0",
            "1e-130",
            "12345678901234567890123456789012345678",
            "9.9999999999999999999999999999999999999e125",
        ]
        .into_iter()
        .map(|s| s.parse().unwrap())
        .collect();
        for sequence in &values {
            let record = Reading {
                group: "readings".into(),
                sequence: *sequence,
                visible: true,
            };
            let put = table.put(&record).build().unwrap();
            put.decode(put.apply(client.put_item()).send().await)
                .unwrap();
        }
        for order in [QueryOrder::Ascending, QueryOrder::Descending] {
            let mut cursor: Option<Vec<u8>> = None;
            let mut seen = Vec::new();
            for _ in 0..=values.len() {
                let request = {
                    let mut query = table
                        .query(Reading::primary().partition("readings".into()), Strong)
                        .order(order)
                        .limit(limit(1));
                    if let Some(bytes) = cursor.take() {
                        query = query.resume(Cursor::from_bytes(&bytes).unwrap());
                    }
                    query.build().unwrap()
                };
                let raw = request.apply(client.query()).send().await.unwrap();
                let page = numeric_page(&request, raw);
                assert!(page.count <= 1);
                seen.extend(page.items.into_iter().map(|row| row.sequence));
                let Some(next) = page.next else {
                    break;
                };
                cursor = Some(next.to_bytes().unwrap());
            }
            assert!(
                cursor.is_none(),
                "query must terminate within the page bound"
            );
            let mut expected = values.clone();
            if order == QueryOrder::Descending {
                expected.reverse();
            }
            assert!(seen == expected, "query must preserve exact numeric order");
        }
        let filtered = || {
            table
                .query(Reading::primary().partition("readings".into()), Strong)
                .filter(ReadingAttrs::VISIBLE.eq(false))
        };
        let request = filtered().limit(limit(1)).build().unwrap();
        let page = numeric_page(
            &request,
            request.apply(client.query()).send().await.unwrap(),
        );
        assert_eq!((page.count, page.scanned_count), (0, 1));
        assert!(page.items.is_empty());
        let bytes = page.next.unwrap().to_bytes().unwrap();
        assert!(matches!(
            table
                .query(Reading::primary().partition("readings".into()), Strong)
                .resume(Cursor::from_bytes(&bytes).unwrap())
                .build(),
            Err(BuildError::CursorMismatch)
        ));
        let request = filtered()
            .limit(limit(2))
            .resume(Cursor::from_bytes(&bytes).unwrap())
            .build()
            .unwrap();
        let page = numeric_page(
            &request,
            request.apply(client.query()).send().await.unwrap(),
        );
        assert_eq!((page.count, page.scanned_count), (0, 2));
        assert!(page.next.is_some());
    })
    .await;
}

#[derive(tabulary::Document)]
#[tabulary(rename_all = "snake_case")]
struct Postal {
    city: String,
    attachment: Vec<u8>,
}

#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Envelope {
    #[tabulary(partition_key)]
    envelope_ref: String,
    quantity: u64,
    postal: Postal,
    #[tabulary(ttl)]
    purge_on: Option<TtlSeconds>,
}

#[derive(tabulary::Projection)]
#[tabulary(record = Envelope)]
struct Quantity {
    #[tabulary(attribute = EnvelopeAttrs::QUANTITY)]
    quantity: u64,
}

#[tokio::test]
#[ignore = "requires explicit TABULARY_DYNAMODB_ENDPOINT, AWS region and credentials"]
async fn parallel_scan_segments_resume_projected_pages_and_store_ttl_and_nested_values() {
    super::support::with_table::<Envelope, _, _>(|client, name| async move {
        let table = Table::<Envelope>::new(name).unwrap();
        for n in 0..12 {
            let row = Envelope {
                envelope_ref: format!("envelope-{n}"),
                quantity: n,
                postal: Postal {
                    city: "Exampleton".into(),
                    attachment: vec![0, 255],
                },
                purge_on: Some(TtlSeconds::new(86_400)),
            };
            let item = row.to_item().unwrap();
            let put = table.put(&row).build().unwrap();
            put.decode(put.apply(client.put_item()).send().await)
                .unwrap();
            let get = table.get(row.key().unwrap(), Strong).build().unwrap();
            let raw = get.apply(client.get_item()).send().await.unwrap();
            assert_eq!(raw.item().unwrap()["purge_on"], Av::N("86400".into()));
            assert_eq!(
                get.decode(Ok(raw))
                    .unwrap()
                    .item
                    .unwrap()
                    .to_item()
                    .unwrap(),
                item
            );
        }
        let scan_segment = async |segment| {
            let mut seen = BTreeSet::new();
            let mut continuations = 0;
            let mut cursor: Option<Vec<u8>> = None;
            for _ in 0..=12 {
                let request = {
                    let mut scan = table
                        .scan(Strong)
                        .segment(ScanSegment::new(segment, 3).unwrap())
                        .select::<Quantity>()
                        .limit(limit(1));
                    if let Some(bytes) = cursor.take() {
                        scan = scan.resume(Cursor::from_bytes(&bytes).unwrap());
                    }
                    scan.build().unwrap()
                };
                let raw = request.apply(client.scan()).send().await.unwrap();
                for item in raw.items() {
                    assert_eq!(item.len(), 1);
                    assert!(item.contains_key(EnvelopeAttrs::QUANTITY.name()));
                }
                let page = request.decode(Ok(raw)).unwrap();
                for view in page.items {
                    assert!(seen.insert(view.quantity), "segment must not repeat rows");
                }
                let Some(next) = page.next else {
                    break;
                };
                let bytes = next.to_bytes().unwrap();
                assert!(matches!(
                    table
                        .scan(Strong)
                        .segment(ScanSegment::new((segment + 1) % 3, 3).unwrap())
                        .select::<Quantity>()
                        .resume(Cursor::from_bytes(&bytes).unwrap())
                        .build(),
                    Err(BuildError::CursorMismatch)
                ));
                cursor = Some(bytes);
                continuations += 1;
            }
            assert!(
                cursor.is_none(),
                "segment must terminate within the page bound"
            );
            (seen, continuations)
        };
        let (a, b, c) = tokio::join!(scan_segment(0), scan_segment(1), scan_segment(2));
        let mut all = BTreeSet::new();
        let mut continuations = 0;
        for (seen, count) in [a, b, c] {
            for value in seen {
                assert!(all.insert(value), "scan segments must not overlap");
            }
            continuations += count;
        }
        assert_eq!(all, (0..12).collect());
        assert!(continuations > 0);
        let ttl = Envelope::table_definition()
            .ttl_input(table.name())
            .unwrap()
            .unwrap();
        let output = client
            .update_time_to_live()
            .set_table_name(ttl.table_name)
            .set_time_to_live_specification(ttl.time_to_live_specification)
            .send()
            .await
            .unwrap();
        assert_eq!(
            output
                .time_to_live_specification()
                .unwrap()
                .attribute_name(),
            "purge_on"
        );
        timeout(Duration::from_secs(60), async {
            loop {
                let output = client
                    .describe_time_to_live()
                    .table_name(table.name())
                    .send()
                    .await
                    .unwrap();
                let description = output.time_to_live_description().unwrap();
                if description.time_to_live_status()
                    == Some(&aws_sdk_dynamodb::types::TimeToLiveStatus::Enabled)
                {
                    assert_eq!(description.attribute_name(), Some("purge_on"));
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("TTL activation deadline");
    })
    .await;
}
