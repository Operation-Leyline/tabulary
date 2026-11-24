#![cfg(feature = "derive")]
//! Strict DynamoDB Streams image decoding for records and multi-object tables.
use aws_sdk_dynamodb::types::AttributeValue as Av;
use serde_json::{Value as Json, json};
use tabulary::{
    DecodeFault, Item, KeysOnlyChange, MultiKey, NewImageChange, OldImageChange, Record,
    StreamChange, StreamDecoder, StreamError, StreamEventName, StreamImage, StreamImages,
    decode_multi_object_image, decode_multi_object_key, decode_record_image, decode_record_key,
};

#[allow(dead_code)]
#[path = "../../../docs/examples/08_multi_object.rs"]
mod example;
use example::{OrderAudit, OrderHeader, OrderHeaderKey, OrderLine, OrderLineKey, OrderTable};

#[derive(Debug, PartialEq, Record)]
#[tabulary(rename_all = "snake_case")]
struct Session {
    #[tabulary(partition_key)]
    session_ref: String,
    #[tabulary(sort_key)]
    seq: u64,
    status: String,
}

fn session(status: &str) -> Session {
    Session {
        session_ref: "ses-moss".into(),
        seq: 2,
        status: status.into(),
    }
}
fn keys_of(item: &Item, names: &[&str]) -> Item {
    names
        .iter()
        .map(|name| (name.to_string(), item[*name].clone()))
        .collect()
}
fn session_keys() -> Item {
    keys_of(&session("open").to_item().unwrap(), &["session_ref", "seq"])
}
fn sessions() -> StreamDecoder<Session, SessionKey> {
    StreamDecoder::record()
}
fn orders() -> StreamDecoder<OrderTable, MultiKey<OrderTable>> {
    StreamDecoder::multi_object()
}
fn images<'a>(
    event: StreamEventName,
    keys: &'a Item,
    old_image: Option<&'a Item>,
    new_image: Option<&'a Item>,
) -> StreamImages<'a> {
    StreamImages {
        event,
        keys,
        old_image,
        new_image,
    }
}
fn header() -> OrderHeader {
    OrderHeader {
        order_ref: "ord-teal".into(),
        customer_ref: "cust-sand".into(),
    }
}
fn line(line_no: u64, quantity: u64) -> OrderLine {
    OrderLine {
        order_ref: "ord-teal".into(),
        line_no,
        quantity,
    }
}

/// Convert one stream record's DynamoDB JSON map, as delivered to a consumer,
/// into SDK attribute values.
fn item(json: &Json) -> Item {
    json.as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.clone(), value_of(value)))
        .collect()
}
fn value_of(json: &Json) -> Av {
    let (tag, value) = json.as_object().unwrap().iter().next().unwrap();
    match tag.as_str() {
        "S" => Av::S(value.as_str().unwrap().into()),
        "N" => Av::N(value.as_str().unwrap().into()),
        "BOOL" => Av::Bool(value.as_bool().unwrap()),
        "NULL" => Av::Null(true),
        "M" => Av::M(item(value)),
        "L" => Av::L(value.as_array().unwrap().iter().map(value_of).collect()),
        other => panic!("fixture uses unsupported tag {other}"),
    }
}

#[test]
fn single_images_and_keys_decode_strictly() {
    let raw = session("open").to_item().unwrap();
    assert_eq!(
        decode_record_image::<Session>(&raw).unwrap(),
        session("open")
    );
    let key: SessionKey = decode_record_key::<Session, _>(&session_keys()).unwrap();
    assert_eq!((key.session_ref.as_str(), key.seq), ("ses-moss", 2));

    // A full image is not a key: Keys holds the primary attributes only.
    let error = decode_record_key::<Session, SessionKey>(&raw)
        .err()
        .unwrap();
    assert_eq!(error.record, "Session");
    assert_eq!(error.attribute(), Some("status"));
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);

    let line = line(4, 2).to_item().unwrap();
    assert!(matches!(
        decode_multi_object_image::<OrderTable>(&line).unwrap(),
        OrderTable::Line(l) if l.line_no == 4
    ));
    let key = decode_multi_object_key::<OrderTable>(&keys_of(&line, &["pk", "sk"])).unwrap();
    assert_eq!(key.variant().name, "Line");
    assert_eq!(key.item().len(), 2);
    let member: OrderLineKey = key.member::<OrderLine, _>().unwrap().unwrap();
    assert_eq!((member.order_ref.as_str(), member.line_no), ("ord-teal", 4));
    assert!(
        key.member::<OrderHeader, OrderHeaderKey>()
            .unwrap()
            .is_none()
    );
    assert_eq!(
        format!("{key:?}"),
        r#"MultiKey { variant: "Line", item: "[redacted]" }"#
    );
}

#[test]
fn multi_object_keys_fail_closed() {
    let mut keys = keys_of(&header().to_item().unwrap(), &["pk", "sk"]);
    keys.insert("sk".into(), Av::S("REFUND#1".into()));
    let error = decode_multi_object_key::<OrderTable>(&keys).err().unwrap();
    assert_eq!(error.record, "OrderTable");
    assert_eq!(error.attribute(), Some("sk"));
    assert_eq!(error.kind, DecodeFault::UnknownVariant);

    keys.insert("sk".into(), Av::N("1".into()));
    let error = decode_multi_object_key::<OrderTable>(&keys).err().unwrap();
    assert!(matches!(error.kind, DecodeFault::WrongType { .. }));

    keys.insert("sk".into(), Av::S(String::new()));
    let error = decode_multi_object_key::<OrderTable>(&keys).err().unwrap();
    assert!(matches!(error.kind, DecodeFault::KeyFormat(_)));

    keys.remove("sk");
    let error = decode_multi_object_key::<OrderTable>(&keys).err().unwrap();
    assert_eq!(
        (error.attribute(), error.kind),
        (Some("sk"), DecodeFault::Missing)
    );

    let mut keys = keys_of(&header().to_item().unwrap(), &["pk", "sk"]);
    keys.insert("customer_ref".into(), Av::S("cust-sand".into()));
    let error = decode_multi_object_key::<OrderTable>(&keys).err().unwrap();
    assert_eq!(error.attribute(), Some("customer_ref"));
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);

    // A member key that its own template refuses fails when decoded as that member.
    let mut keys = keys_of(&line(1, 1).to_item().unwrap(), &["pk", "sk"]);
    keys.insert("sk".into(), Av::S("LINE#x".into()));
    let key = decode_multi_object_key::<OrderTable>(&keys).unwrap();
    let error = key.member::<OrderLine, OrderLineKey>().err().unwrap();
    assert!(matches!(error.kind, DecodeFault::KeyFormat(_)));
}

#[test]
fn record_insert_modify_and_remove() {
    let keys = session_keys();
    let (open, closed) = (
        session("open").to_item().unwrap(),
        session("closed").to_item().unwrap(),
    );
    use StreamEventName::*;
    assert_eq!(
        images(Insert, &keys, None, Some(&open))
            .new_and_old(sessions())
            .unwrap(),
        StreamChange::Insert(session("open"))
    );
    assert_eq!(
        images(Modify, &keys, Some(&open), Some(&closed))
            .new_and_old(sessions())
            .unwrap(),
        StreamChange::Modify {
            old: session("open"),
            new: session("closed"),
        }
    );
    assert_eq!(
        images(Remove, &keys, Some(&closed), None)
            .new_and_old(sessions())
            .unwrap(),
        StreamChange::Remove(session("closed"))
    );
}

#[test]
fn record_single_image_and_keys_only_views() {
    let keys = session_keys();
    let open = session("open").to_item().unwrap();
    use StreamEventName::*;

    let change = images(Modify, &keys, None, Some(&open))
        .new_image(sessions())
        .unwrap();
    assert!(matches!(change, NewImageChange::Modify(s) if s == session("open")));
    let change = images(Remove, &keys, None, None)
        .new_image(sessions())
        .unwrap();
    assert!(matches!(change, NewImageChange::Remove(k) if k.seq == 2));

    let change = images(Insert, &keys, None, None)
        .old_image(sessions())
        .unwrap();
    assert!(matches!(change, OldImageChange::Insert(k) if k.session_ref == "ses-moss"));
    let change = images(Remove, &keys, Some(&open), None)
        .old_image(sessions())
        .unwrap();
    assert!(matches!(change, OldImageChange::Remove(s) if s == session("open")));

    for (event, expected) in [(Insert, "Insert"), (Modify, "Modify"), (Remove, "Remove")] {
        let change = images(event, &keys, None, None)
            .keys_only(sessions())
            .unwrap();
        let (name, key) = match change {
            KeysOnlyChange::Insert(k) => ("Insert", k),
            KeysOnlyChange::Modify(k) => ("Modify", k),
            KeysOnlyChange::Remove(k) => ("Remove", k),
        };
        assert_eq!((name, key.seq), (expected, 2));
    }
}

#[test]
fn images_the_view_does_not_promise_are_refused() {
    let keys = session_keys();
    let open = session("open").to_item().unwrap();
    use StreamEventName::*;
    let cases = [
        (
            images(Modify, &keys, None, Some(&open)).new_and_old(sessions()),
            StreamError::MissingImage {
                image: StreamImage::OldImage,
            },
        ),
        (
            images(Insert, &keys, Some(&open), Some(&open)).new_and_old(sessions()),
            StreamError::UnexpectedImage {
                image: StreamImage::OldImage,
            },
        ),
        (
            images(Remove, &keys, Some(&open), Some(&open)).new_and_old(sessions()),
            StreamError::UnexpectedImage {
                image: StreamImage::NewImage,
            },
        ),
        (
            images(Remove, &keys, None, None).new_and_old(sessions()),
            StreamError::MissingImage {
                image: StreamImage::OldImage,
            },
        ),
    ];
    for (result, expected) in cases {
        assert_eq!(result.err().unwrap(), expected);
    }
    assert_eq!(
        images(Insert, &keys, None, None)
            .new_image(sessions())
            .err()
            .unwrap(),
        StreamError::MissingImage {
            image: StreamImage::NewImage
        }
    );
    assert_eq!(
        images(Modify, &keys, Some(&open), Some(&open))
            .new_image(sessions())
            .err()
            .unwrap(),
        StreamError::UnexpectedImage {
            image: StreamImage::OldImage
        }
    );
    assert_eq!(
        images(Insert, &keys, None, Some(&open))
            .old_image(sessions())
            .err()
            .unwrap(),
        StreamError::UnexpectedImage {
            image: StreamImage::NewImage
        }
    );
    assert_eq!(
        images(Modify, &keys, None, Some(&open))
            .keys_only(sessions())
            .err()
            .unwrap(),
        StreamError::UnexpectedImage {
            image: StreamImage::NewImage
        }
    );
    assert_eq!(
        StreamEventName::from_wire("UPSERT").err().unwrap(),
        StreamError::UnknownEventName
    );
    assert_eq!(StreamEventName::from_wire("MODIFY").unwrap(), Modify);
}

#[test]
fn corrupt_attributes_and_keys_fail_closed() {
    let keys = session_keys();
    let open = session("open").to_item().unwrap();
    let mut corrupt = open.clone();
    corrupt.insert("status".into(), Av::N("1".into()));
    use StreamEventName::*;

    let error = images(Modify, &keys, Some(&open), Some(&corrupt))
        .new_and_old(sessions())
        .err()
        .unwrap();
    let StreamError::Decode { image, source } = &error else {
        panic!("expected a decode error, got {error:?}")
    };
    assert_eq!(*image, StreamImage::NewImage);
    assert_eq!(
        (source.record, source.attribute()),
        ("Session", Some("status"))
    );
    assert!(matches!(source.kind, DecodeFault::WrongType { .. }));
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(
        error.to_string(),
        format!("NewImage: {source}"),
        "Display names the image and the attribute, never the value"
    );

    let mut missing = open.clone();
    missing.remove("status");
    let error = images(Remove, &keys, Some(&missing), None)
        .old_image(sessions())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::OldImage, ref source }
            if source.kind == DecodeFault::Missing
    ));

    // An image whose key differs from the record's Keys.
    let other = Session {
        seq: 3,
        ..session("open")
    }
    .to_item()
    .unwrap();
    assert_eq!(
        images(Modify, &keys, Some(&open), Some(&other))
            .new_and_old(sessions())
            .err()
            .unwrap(),
        StreamError::KeyMismatch {
            image: StreamImage::NewImage,
            attribute: "seq",
        }
    );

    // Keys hold primary attributes only, all of them, correctly typed.
    let mut extra = keys.clone();
    extra.insert("status".into(), Av::S("open".into()));
    let error = images(Insert, &extra, None, Some(&open))
        .new_and_old(sessions())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::Keys, ref source }
            if source.kind == DecodeFault::UnknownAttribute
    ));
    let mut short = keys.clone();
    short.remove("seq");
    let error = images(Insert, &short, None, Some(&open))
        .new_and_old(sessions())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::Keys, ref source }
            if source.kind == DecodeFault::Missing && source.attribute() == Some("seq")
    ));
    let mut mistyped = keys;
    mistyped.insert("seq".into(), Av::S("2".into()));
    let error = images(Remove, &mistyped, None, None)
        .keys_only(sessions())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::Keys, ref source }
            if matches!(source.kind, DecodeFault::WrongType { .. })
    ));
}

#[test]
fn multi_object_changes_decode_each_member() {
    use StreamEventName::*;
    let header_item = header().to_item().unwrap();
    let header_keys = keys_of(&header_item, &["pk", "sk"]);
    let change = images(Insert, &header_keys, None, Some(&header_item))
        .new_and_old(orders())
        .unwrap();
    assert!(
        matches!(change, StreamChange::Insert(OrderTable::Header(h)) if h.customer_ref == "cust-sand")
    );

    let (before, after) = (line(1, 2).to_item().unwrap(), line(1, 5).to_item().unwrap());
    let line_keys = keys_of(&before, &["pk", "sk"]);
    let change = images(Modify, &line_keys, Some(&before), Some(&after))
        .new_and_old(orders())
        .unwrap();
    assert!(matches!(
        change,
        StreamChange::Modify {
            old: OrderTable::Line(ref old),
            new: OrderTable::Line(ref new),
        } if old.quantity == 2 && new.quantity == 5
    ));

    let audit = OrderAudit {
        order_ref: "ord-teal".into(),
        seq: 9,
        note: "shipped".into(),
    }
    .to_item()
    .unwrap();
    let audit_keys = keys_of(&audit, &["pk", "sk"]);
    let change = images(Remove, &audit_keys, Some(&audit), None)
        .new_and_old(orders())
        .unwrap();
    assert!(matches!(change, StreamChange::Remove(OrderTable::Audit(a)) if a.seq == 9));

    let change = images(Remove, &line_keys, None, None)
        .new_image(orders())
        .unwrap();
    let NewImageChange::Remove(key) = change else {
        panic!("expected a removal key")
    };
    assert_eq!(key.variant().name, "Line");
    let line_key: OrderLineKey = key.member::<OrderLine, _>().unwrap().unwrap();
    assert_eq!(line_key.line_no, 1);

    let change = images(Modify, &audit_keys, None, None)
        .keys_only(orders())
        .unwrap();
    assert!(matches!(change, KeysOnlyChange::Modify(ref k) if k.variant().name == "Audit"));
}

#[test]
fn multi_object_changes_fail_closed() {
    use StreamEventName::*;
    let header_item = header().to_item().unwrap();
    let line_item = line(1, 2).to_item().unwrap();
    let header_keys = keys_of(&header_item, &["pk", "sk"]);

    // Old and new images select different members: refused before key checks.
    assert_eq!(
        images(Modify, &header_keys, Some(&header_item), Some(&line_item))
            .new_and_old(orders())
            .err()
            .unwrap(),
        StreamError::VariantMismatch {
            old: "Header",
            new: "Line",
        }
    );

    // Same member, different primary key.
    let other_line = line(2, 2).to_item().unwrap();
    let line_keys = keys_of(&line_item, &["pk", "sk"]);
    assert_eq!(
        images(Modify, &line_keys, Some(&line_item), Some(&other_line))
            .new_and_old(orders())
            .err()
            .unwrap(),
        StreamError::KeyMismatch {
            image: StreamImage::NewImage,
            attribute: "sk",
        }
    );

    // A key no member declares.
    let mut unknown = header_item.clone();
    unknown.insert("sk".into(), Av::S("REFUND#1".into()));
    let unknown_keys = keys_of(&unknown, &["pk", "sk"]);
    let error = images(Insert, &unknown_keys, None, Some(&unknown))
        .new_and_old(orders())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::NewImage, ref source }
            if source.kind == DecodeFault::UnknownVariant && source.attribute() == Some("sk")
    ));
    let error = images(Remove, &unknown_keys, None, None)
        .keys_only(orders())
        .err()
        .unwrap();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::Keys, ref source }
            if source.kind == DecodeFault::UnknownVariant
    ));

    // A corrupt attribute names the enum, the member, and the attribute.
    let mut corrupt = line_item.clone();
    corrupt.insert("quantity".into(), Av::S("two".into()));
    let error = images(Modify, &line_keys, Some(&corrupt), Some(&line_item))
        .new_and_old(orders())
        .err()
        .unwrap();
    let StreamError::Decode { image, source } = error else {
        panic!("expected a decode error")
    };
    assert_eq!(image, StreamImage::OldImage);
    assert_eq!(
        (source.record, source.document, source.attribute()),
        ("OrderTable", "OrderLine", Some("quantity"))
    );
}

/// A `NEW_AND_OLD_IMAGES` record in the service's DynamoDB JSON shape.
#[test]
fn stream_record_fixture_decodes_end_to_end() {
    let record = json!({
        "eventName": "MODIFY",
        "dynamodb": {
            "Keys": {"pk": {"S": "ORD#ord-teal"}, "sk": {"S": "LINE#00000000000000000003"}},
            "OldImage": {
                "pk": {"S": "ORD#ord-teal"}, "sk": {"S": "LINE#00000000000000000003"},
                "order_ref": {"S": "ord-teal"}, "line_no": {"N": "3"}, "quantity": {"N": "1"}
            },
            "NewImage": {
                "pk": {"S": "ORD#ord-teal"}, "sk": {"S": "LINE#00000000000000000003"},
                "order_ref": {"S": "ord-teal"}, "line_no": {"N": "3"}, "quantity": {"N": "4"}
            },
            "StreamViewType": "NEW_AND_OLD_IMAGES"
        }
    });
    let body = &record["dynamodb"];
    let (keys, old, new) = (
        item(&body["Keys"]),
        item(&body["OldImage"]),
        item(&body["NewImage"]),
    );
    let images = StreamImages {
        event: StreamEventName::from_wire(record["eventName"].as_str().unwrap()).unwrap(),
        keys: &keys,
        old_image: Some(&old),
        new_image: Some(&new),
    };
    let change = images.new_and_old(orders()).unwrap();
    let StreamChange::Modify {
        old: OrderTable::Line(old),
        new: OrderTable::Line(new),
    } = change
    else {
        panic!("expected a line modification")
    };
    assert_eq!((old.line_no, old.quantity, new.quantity), (3, 1, 4));
    assert_eq!(
        format!("{images:?}"),
        r#"StreamImages { event: Modify, keys: "[redacted]", old_image: Some("[redacted]"), new_image: Some("[redacted]") }"#
    );
}
