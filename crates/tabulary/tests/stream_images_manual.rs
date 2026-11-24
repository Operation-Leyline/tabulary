//! Stream image decoding with hand-written contracts, run without the derive feature.
use aws_sdk_dynamodb::types::AttributeValue as Av;
use tabulary::{
    DecodeContext, DecodeError, DecodeFault, Document, DynamoType, EncodeContext, EncodeError,
    FieldAccess, FieldDescriptor, Item, KeyDefinition, KeyFormatFault, KeyPattern, KeyPatterns,
    KeysOnlyChange, MemberOf, MultiObject, MultiVariant, PathSegment, Presence, Record, RecordKey,
    RecordSchema, StreamChange, StreamDecoder, StreamError, StreamEventName, StreamImage,
    StreamImages,
};

const PK: FieldDescriptor = FieldDescriptor {
    name: "pk",
    index: 0,
    presence: Presence::Required,
    access: FieldAccess::KeyOnly,
    wire: DynamoType::S,
};
const STATE: FieldDescriptor = FieldDescriptor {
    name: "state",
    index: 1,
    presence: Presence::Required,
    access: FieldAccess::Mutable,
    wire: DynamoType::S,
};
const PK_ONLY: KeyDefinition = KeyDefinition {
    partition: 0,
    sort: None,
};

/// A record stored as `{pk: prefix + id, state}`; its key is the id.
macro_rules! prefixed {
    ($record:ident, $key:ident, $prefix:literal) => {
        #[derive(Debug, PartialEq)]
        struct $record {
            id: String,
            state: String,
        }
        #[derive(Debug, PartialEq)]
        struct $key(String);
        impl Record for $record {
            const SCHEMA: RecordSchema = RecordSchema {
                name: stringify!($record),
                fields: &[PK, STATE],
                primary: PK_ONLY,
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
                Ok([
                    ("pk".into(), Av::S(format!("{}{}", $prefix, self.id))),
                    ("state".into(), Av::S(self.state.clone())),
                ]
                .into())
            }
            fn decode_map(item: &Item, cx: &mut DecodeContext) -> Result<Self, DecodeError> {
                let id = $key::decode_key(item)?.0;
                let at = cx.at(PathSegment::Attribute("state".into()));
                let state = match item.get("state") {
                    Some(Av::S(state)) => state.clone(),
                    Some(_) => return Err(at.error(DecodeFault::UnsupportedType)),
                    None => return Err(at.error(DecodeFault::Missing)),
                };
                if item.len() != 2 {
                    return Err(cx.error(DecodeFault::UnknownAttribute));
                }
                Ok(Self { id, state })
            }
        }
        impl RecordKey<$record> for $key {
            fn encode_key(&self) -> Result<Item, EncodeError> {
                Ok([("pk".into(), Av::S(format!("{}{}", $prefix, self.0)))].into())
            }
            fn decode_key(item: &Item) -> Result<Self, DecodeError> {
                let cx =
                    DecodeContext::new(stringify!($record)).at(PathSegment::Attribute("pk".into()));
                match item.get("pk") {
                    Some(Av::S(key)) => key
                        .strip_prefix($prefix)
                        .map(|id| Self(id.into()))
                        .ok_or_else(|| {
                            cx.error(DecodeFault::KeyFormat(KeyFormatFault::InvalidPrefix))
                        }),
                    _ => Err(cx.error(DecodeFault::Missing)),
                }
            }
        }
    };
}
prefixed!(Session, SessionKey, "SES#");
prefixed!(Device, DeviceKey, "DEV#");

#[derive(Debug, PartialEq)]
enum Access {
    Session(Session),
    Device(Device),
}
pub struct AccessSeal<R>(std::marker::PhantomData<R>);
impl MultiObject for Access {
    type MemberSeal<R> = AccessSeal<R>;
    const SCHEMA: RecordSchema = RecordSchema {
        name: "Access",
        fields: &[PK],
        primary: PK_ONLY,
        indexes: &[],
        ttl: None,
    };
    const VARIANTS: &'static [MultiVariant] = &[
        MultiVariant {
            name: "Session",
            record: "Session",
            keys: Session::KEY_PATTERNS,
        },
        MultiVariant {
            name: "Device",
            record: "Device",
            keys: Device::KEY_PATTERNS,
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
    const SEAL: AccessSeal<Self> = AccessSeal(std::marker::PhantomData);
    fn into_object(self) -> Access {
        Access::Session(self)
    }
}
impl MemberOf<Access> for Device {
    const VARIANT: usize = 1;
    const SEAL: AccessSeal<Self> = AccessSeal(std::marker::PhantomData);
    fn into_object(self) -> Access {
        Access::Device(self)
    }
}
const _: () = assert!(tabulary::multi_object_fault::<Access>().is_none());
const _: () = assert!(tabulary::member_fault::<Access, Session>().is_none());
const _: () = assert!(tabulary::member_fault::<Access, Device>().is_none());

fn row(pk: &str, state: &str) -> Item {
    [
        ("pk".into(), Av::S(pk.into())),
        ("state".into(), Av::S(state.into())),
    ]
    .into()
}
fn key(pk: &str) -> Item {
    [("pk".into(), Av::S(pk.into()))].into()
}
fn session(state: &str) -> Session {
    Session {
        id: "moss".into(),
        state: state.into(),
    }
}

#[test]
fn manual_record_changes() {
    use StreamEventName::*;
    let decoder = StreamDecoder::<Session, SessionKey>::record();
    let keys = key("SES#moss");
    let (open, closed) = (row("SES#moss", "open"), row("SES#moss", "closed"));
    let images = |event, old_image, new_image| StreamImages {
        event,
        keys: &keys,
        old_image,
        new_image,
    };
    assert_eq!(
        images(Insert, None, Some(&open)).new_and_old(decoder),
        Ok(StreamChange::Insert(session("open")))
    );
    assert_eq!(
        images(Modify, Some(&open), Some(&closed)).new_and_old(decoder),
        Ok(StreamChange::Modify {
            old: session("open"),
            new: session("closed"),
        })
    );
    assert_eq!(
        images(Remove, Some(&closed), None).new_and_old(decoder),
        Ok(StreamChange::Remove(session("closed")))
    );
    assert_eq!(
        images(Remove, None, None).keys_only(decoder),
        Ok(KeysOnlyChange::Remove(SessionKey("moss".into())))
    );

    let mut corrupt = open.clone();
    corrupt.insert("state".into(), Av::Bool(true));
    let error = images(Insert, None, Some(&corrupt))
        .new_and_old(decoder)
        .unwrap_err();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::NewImage, ref source }
            if source.record == "Session" && source.attribute() == Some("state")
    ));
}

#[test]
fn manual_multi_object_changes() {
    use StreamEventName::*;
    let decoder = StreamDecoder::<Access, _>::multi_object();
    let device_keys = key("DEV#slate");
    let (before, after) = (row("DEV#slate", "paired"), row("DEV#slate", "lost"));
    let change = StreamImages {
        event: Modify,
        keys: &device_keys,
        old_image: Some(&before),
        new_image: Some(&after),
    }
    .new_and_old(decoder)
    .unwrap();
    assert!(matches!(
        change,
        StreamChange::Modify { old: Access::Device(ref o), new: Access::Device(ref n) }
            if o.state == "paired" && n.state == "lost"
    ));

    let session_row = row("SES#moss", "open");
    assert_eq!(
        StreamImages {
            event: Modify,
            keys: &device_keys,
            old_image: Some(&before),
            new_image: Some(&session_row),
        }
        .new_and_old(decoder)
        .unwrap_err(),
        StreamError::VariantMismatch {
            old: "Device",
            new: "Session",
        }
    );

    let unknown = key("ORD#teal");
    let error = StreamImages {
        event: Insert,
        keys: &unknown,
        old_image: None,
        new_image: None,
    }
    .keys_only(decoder)
    .unwrap_err();
    assert!(matches!(
        error,
        StreamError::Decode { image: StreamImage::Keys, ref source }
            if source.record == "Access" && source.kind == DecodeFault::UnknownVariant
    ));

    let KeysOnlyChange::Insert(key) = StreamImages {
        event: Insert,
        keys: &device_keys,
        old_image: None,
        new_image: None,
    }
    .keys_only(decoder)
    .unwrap() else {
        panic!("expected an insertion key")
    };
    assert_eq!(key.variant().name, "Device");
    assert_eq!(
        key.member::<Device, DeviceKey>(),
        Ok(Some(DeviceKey("slate".into())))
    );
    assert_eq!(key.member::<Session, SessionKey>(), Ok(None));
}
