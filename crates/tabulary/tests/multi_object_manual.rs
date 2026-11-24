//! Hand-written multi-object contracts, compiled and run without the derive feature.
use aws_sdk_dynamodb::{operation::scan::ScanOutput, types::AttributeValue as Av};
use tabulary::{
    BuildError, DecodeContext, DecodeError, DecodeFault, Document, DynamoType, EncodeContext,
    EncodeError, FieldAccess, FieldDescriptor, Item, KeyDefinition, KeyFormatFault, KeyPattern,
    KeyPatterns, MemberOf, MultiObject, MultiTable, MultiVariant, PathSegment, Presence,
    ReadConsistency, Record, RecordKey, RecordSchema,
};

const PK: FieldDescriptor = FieldDescriptor {
    name: "pk",
    index: 0,
    presence: Presence::Required,
    access: FieldAccess::KeyOnly,
    wire: DynamoType::S,
};
const KEYS_ONLY: &[FieldDescriptor] = &[PK];

fn schema(name: &'static str) -> RecordSchema {
    RecordSchema {
        name,
        fields: KEYS_ONLY,
        primary: KeyDefinition {
            partition: 0,
            sort: None,
        },
        indexes: &[],
        ttl: None,
    }
}

/// One hand-written record per prefix; the stored key is `prefix + id`.
macro_rules! prefixed {
    ($record:ident, $key:ident, $prefix:literal) => {
        #[derive(Debug, PartialEq)]
        struct $record {
            id: String,
        }
        struct $key(String);
        impl Record for $record {
            const SCHEMA: RecordSchema = RecordSchema {
                name: stringify!($record),
                fields: KEYS_ONLY,
                primary: KeyDefinition {
                    partition: 0,
                    sort: None,
                },
                indexes: &[],
                ttl: None,
            };
            const KEY_PATTERNS: KeyPatterns = KeyPatterns {
                partition: KeyPattern::Prefix($prefix),
                sort: KeyPattern::Any,
            };
        }
        impl Document for $record {
            const NAME: &'static str = stringify!($record);
            fn encode_map(&self, _: &mut EncodeContext) -> Result<Item, EncodeError> {
                Ok([("pk".into(), Av::S(format!("{}{}", $prefix, self.id)))].into())
            }
            fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
                let cx = cx.at(PathSegment::Attribute("pk".into()));
                if item.len() != 1 {
                    return Err(cx.error(DecodeFault::UnknownAttribute));
                }
                match item.get("pk") {
                    Some(Av::S(key)) => key
                        .strip_prefix($prefix)
                        .map(|id| Self { id: id.into() })
                        .ok_or_else(|| {
                            cx.error(DecodeFault::KeyFormat(KeyFormatFault::InvalidPrefix))
                        }),
                    _ => Err(cx.error(DecodeFault::Missing)),
                }
            }
        }
        impl RecordKey<$record> for $key {
            fn encode_key(&self) -> Result<Item, EncodeError> {
                Ok([("pk".into(), Av::S(format!("{}{}", $prefix, self.0)))].into())
            }
            fn decode_key(item: &Item) -> Result<Self, DecodeError> {
                $record::decode_map(item, &mut DecodeContext::new(stringify!($record)))
                    .map(|record| Self(record.id))
            }
        }
    };
}
prefixed!(Session, SessionKey, "SES#");
prefixed!(Device, DeviceKey, "DEV#");
prefixed!(Tablet, TabletKey, "DEV#TAB#");

#[derive(Debug, PartialEq)]
enum Access {
    Session(Session),
    Device(Device),
}
/// Only this module can construct the seal, so only it can declare members.
pub struct AccessSeal(());
impl MultiObject for Access {
    type MemberSeal = AccessSeal;
    const SCHEMA: RecordSchema = RecordSchema {
        name: "Access",
        fields: KEYS_ONLY,
        primary: KeyDefinition {
            partition: 0,
            sort: None,
        },
        indexes: &[],
        ttl: None,
    };
    const VARIANTS: &'static [MultiVariant] = &[
        MultiVariant {
            name: "Session",
            record: "Session",
            keys: KeyPatterns {
                partition: KeyPattern::Prefix("SES#"),
                sort: KeyPattern::Any,
            },
        },
        MultiVariant {
            name: "Device",
            record: "Device",
            keys: KeyPatterns {
                partition: KeyPattern::Prefix("DEV#"),
                sort: KeyPattern::Any,
            },
        },
    ];
    fn decode_variant(
        variant: usize,
        item: &Item,
        cx: &mut DecodeContext,
    ) -> Result<Self, DecodeError> {
        match variant {
            0 => Session::decode_map(item, cx).map(Self::Session),
            1 => Device::decode_map(item, cx).map(Self::Device),
            _ => Err(cx.error(DecodeFault::UnknownVariant)),
        }
    }
}
impl MemberOf<Access> for Session {
    const VARIANT: usize = 0;
    const SEAL: AccessSeal = AccessSeal(());
    fn into_object(self) -> Access {
        Access::Session(self)
    }
}
impl MemberOf<Access> for Device {
    const VARIANT: usize = 1;
    const SEAL: AccessSeal = AccessSeal(());
    fn into_object(self) -> Access {
        Access::Device(self)
    }
}
// The same compile-time checks the derive emits.
const _: () = assert!(tabulary::multi_object_fault::<Access>().is_none());
const _: () = assert!(tabulary::member_fault::<Access, Session>().is_none());
const _: () = assert!(tabulary::member_fault::<Access, Device>().is_none());

/// `DEV#` and `DEV#TAB#` overlap: a key could select either member.
#[derive(Debug)]
struct Overlapping;
impl MultiObject for Overlapping {
    type MemberSeal = ();
    const SCHEMA: RecordSchema = RecordSchema {
        name: "Overlapping",
        ..Access::SCHEMA
    };
    const VARIANTS: &'static [MultiVariant] = &[
        Access::VARIANTS[1],
        MultiVariant {
            name: "Tablet",
            record: "Tablet",
            keys: KeyPatterns {
                partition: KeyPattern::Prefix("DEV#TAB#"),
                sort: KeyPattern::Any,
            },
        },
    ];
    fn decode_variant(_: usize, _: &Item, _: &mut DecodeContext) -> Result<Self, DecodeError> {
        unreachable!("an invalid multi-object contract never dispatches")
    }
}
// A hand-written membership for a type the enum does not hold, written inside
// the trusted declaring module (other modules cannot construct `AccessSeal`).
// Its keys fit the Device pattern, but the record check still refuses it, and
// every `MultiTable` member operation asserts that check at compile time.
impl MemberOf<Access> for Tablet {
    const VARIANT: usize = 1;
    const SEAL: AccessSeal = AccessSeal(());
    fn into_object(self) -> Access {
        Access::Device(Device { id: self.id })
    }
}

fn pk(text: &str) -> Item {
    [("pk".into(), Av::S(text.into()))].into()
}

#[test]
fn manual_contracts_dispatch_and_fail_closed() {
    assert_eq!(
        tabulary::decode_object::<Access>(&pk("SES#a1")).unwrap(),
        Access::Session(Session { id: "a1".into() })
    );
    assert_eq!(
        tabulary::decode_object::<Access>(&pk("DEV#d1")).unwrap(),
        Access::Device(Device { id: "d1".into() })
    );
    let error = tabulary::decode_object::<Access>(&pk("ACC#x")).unwrap_err();
    assert_eq!((error.record, error.attribute()), ("Access", Some("pk")));
    assert_eq!(error.kind, DecodeFault::UnknownVariant);
    let mut extra = pk("SES#a1");
    extra.insert("surplus".into(), Av::Bool(true));
    let error = tabulary::decode_object::<Access>(&extra).unwrap_err();
    assert_eq!((error.record, error.document), ("Access", "Session"));
    assert_eq!(error.kind, DecodeFault::UnknownAttribute);
    assert_eq!(schema("Access"), Access::SCHEMA);
}

#[test]
fn manual_contracts_are_validated_before_use() {
    assert_eq!(
        tabulary::multi_object_fault::<Overlapping>(),
        Some("overlapping_variant_keys")
    );
    assert!(matches!(
        MultiTable::<Overlapping>::new("access"),
        Err(BuildError::OperationConstraint {
            code: "overlapping_variant_keys"
        })
    ));
    let error = tabulary::decode_object::<Overlapping>(&pk("DEV#d1")).unwrap_err();
    assert_eq!(
        error.kind,
        DecodeFault::Custom {
            code: "overlapping_variant_keys"
        }
    );
    assert_eq!(
        tabulary::member_fault::<Access, Tablet>(),
        Some("member_record_mismatch")
    );
}

#[test]
fn manual_members_use_the_shared_table_operations() {
    let access = MultiTable::<Access>::new("access").unwrap();
    let put = access.put(&Session { id: "a1".into() }).build().unwrap();
    assert_eq!(put.input().table_name(), Some("access"));
    assert_eq!(put.input().item().unwrap()["pk"], Av::S("SES#a1".into()));
    let get = access
        .get(DeviceKey("d1".into()), ReadConsistency::Strong)
        .build()
        .unwrap();
    assert_eq!(get.input().key().unwrap()["pk"], Av::S("DEV#d1".into()));
    let scan = access.scan(ReadConsistency::Eventual).build().unwrap();
    let output = ScanOutput::builder()
        .count(2)
        .scanned_count(2)
        .set_items(Some(vec![pk("DEV#d1"), pk("SES#a1")]))
        .build();
    let page = scan.decode(Ok(output)).unwrap();
    assert_eq!(
        page.items,
        [
            Access::Device(Device { id: "d1".into() }),
            Access::Session(Session { id: "a1".into() })
        ]
    );
    let output = ScanOutput::builder()
        .count(1)
        .scanned_count(1)
        .set_items(Some(vec![pk("ACC#x")]))
        .build();
    let Err(tabulary::ReadError::Decode(error)) = scan.decode(Ok(output)) else {
        panic!("an unknown discriminator must fail the page")
    };
    assert_eq!(error.source.kind, DecodeFault::UnknownVariant);
    let _ = Tablet { id: "t".into() }.into_object();
    let _ = TabletKey("t".into()).encode_key().unwrap();
    let _ = SessionKey::decode_key(&pk("SES#a1")).unwrap();
}
