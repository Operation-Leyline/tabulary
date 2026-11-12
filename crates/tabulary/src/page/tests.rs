use super::*;
use crate::{__private as p, KeyOnly, KeySchema, Native, NoSort, Required, Value};
use aws_sdk_dynamodb::types::AttributeValue as Av;

struct Row;
impl Record for Row {
    const SCHEMA: crate::RecordSchema = p::schema(
        "Row",
        &[p::field::<Row, String, Required, KeyOnly, Native<String>>("pk", 0).descriptor()],
        p::keys(0, None),
    );
}
struct Primary;
impl KeySchema<Row> for Primary {
    type Partition = String;
    type PartitionCodec = Native<String>;
    type Sort = NoSort;
    const KEYS: crate::KeyDefinition = Row::SCHEMA.primary;
}
fn binding() -> CursorBinding<Row, Primary> {
    let key = p::partition::<Row, Primary, _, Native<String>>(Ok("p".into()))
        .build()
        .unwrap();
    CursorBinding::query(
        &crate::Table::new("rows").unwrap(),
        None,
        &key,
        None,
        None,
        crate::ReadConsistency::Strong,
        crate::QueryOrder::Ascending,
    )
    .unwrap()
}
fn key() -> Item {
    [("pk".into(), Av::S("private-key".into()))].into()
}
fn row(n: u64) -> Item {
    [("units".into(), Av::N(n.to_string()))].into()
}
fn decode(item: &Item) -> Result<u64, DecodeError> {
    let mut cx = DecodeContext::new("Row").at(PathSegment::Attribute("units".into()));
    let value = item
        .get("units")
        .ok_or_else(|| cx.error(DecodeFault::Missing))?;
    u64::decode_value(value, &mut cx)
}
fn raw(items: Vec<Item>, count: i32, scanned_count: i32, key: Option<Item>) -> RawPage {
    RawPage {
        items,
        metadata: PageMetadata {
            count,
            scanned_count,
            last_evaluated_key: key,
        },
        capacity: Some(ConsumedCapacity::builder().capacity_units(1.5).build()),
    }
}

#[test]
fn empty_pages_preserve_accounting_and_only_empty_keys_end_pagination() {
    let page = raw(vec![], 0, 7, Some(key()))
        .decode(&binding(), decode)
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!((page.count, page.scanned_count), (0, 7));
    assert_eq!(page.capacity.unwrap().capacity_units, Some(1.5));
    assert_eq!(page.next.unwrap().last_evaluated_key(), &key());
    for end in [None, Some(Item::new())] {
        assert!(
            raw(vec![], 0, 0, end)
                .decode(&binding(), decode)
                .unwrap()
                .next
                .is_none()
        );
    }
    let page = raw(vec![row(9), row(1)], 2, 3, None)
        .decode(&binding(), decode)
        .unwrap();
    assert_eq!(page.items, [9, 1]);
}

#[test]
fn corruption_keeps_the_entire_raw_page_and_metadata_without_payload_in_formatting() {
    let rows = vec![row(9), Item::new(), row(1)];
    let fault = raw(rows.clone(), 3, 8, Some(key()))
        .decode(&binding(), decode)
        .unwrap_err();
    assert_eq!(fault.position, 1);
    assert_eq!(fault.source.record, "Row");
    assert_eq!(fault.source.attribute(), Some("units"));
    assert_eq!(fault.source.kind, DecodeFault::Missing);
    assert_eq!(fault.raw_items(), rows);
    assert_eq!(fault.capacity.as_ref().unwrap().capacity_units, Some(1.5));
    let metadata = fault.page.as_ref().unwrap();
    assert_eq!((metadata.count, metadata.scanned_count), (3, 8));
    assert_eq!(metadata.last_evaluated_key(), Some(&key()));
    assert!(!format!("{fault:?} {fault}").contains("private-key"));
    assert!(std::error::Error::source(fault.as_ref()).is_some());
}

#[test]
fn invalid_page_metadata_is_a_structured_failure_with_the_original_values() {
    for (items, count, scanned, key, attribute) in [
        (vec![], -1, 0, None, "Count"),
        (vec![], 1, 2, None, "Count"),
        (vec![row(1)], 1, 0, None, "ScannedCount"),
        (vec![], 0, -1, None, "ScannedCount"),
        (
            vec![],
            0,
            0,
            Some([("pk".into(), Av::Bool(false))].into()),
            "LastEvaluatedKey",
        ),
    ] {
        let fault = raw(items.clone(), count, scanned, key.clone())
            .decode(&binding(), decode)
            .unwrap_err();
        assert_eq!(fault.position, 0);
        assert_eq!(fault.source.attribute(), Some(attribute));
        assert!(matches!(fault.source.kind, DecodeFault::Custom { .. }));
        assert_eq!(fault.raw_items(), items);
        let metadata = fault.page.unwrap();
        assert_eq!((metadata.count, metadata.scanned_count), (count, scanned));
        assert_eq!(metadata.last_evaluated_key(), key.as_ref());
    }
}
