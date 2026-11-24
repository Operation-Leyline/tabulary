//! Strict decoding of DynamoDB Streams images into records and multi-object enums.
//!
//! A stream record carries an event name, the item's primary key, and up to
//! two full images, depending on the stream's view type. The caller converts
//! each image to an [`Item`]; nothing here reads a stream or calls a service.
use std::{error::Error, fmt, marker::PhantomData};

use aws_sdk_dynamodb::types::AttributeValue;

use crate::multi_object::{multi_object_fault, select_variant};
use crate::{
    DecodeContext, DecodeError, DecodeFault, Document, DynamoType, Item, KeyFormatFault, MemberOf,
    MultiObject, MultiVariant, PathSegment, Record, RecordKey, RecordSchema, decode_object,
};

/// Decode one full stream image strictly as `R`, exactly as a `GetItem` item.
pub fn decode_record_image<R: Record + Document>(item: &Item) -> Result<R, DecodeError> {
    R::decode_map(item, &mut DecodeContext::new(R::NAME))
}

/// Decode one full stream image as a member of `M`, exactly as a polymorphic
/// query row: a key that selects no member is `UnknownVariant`.
pub fn decode_multi_object_image<M: MultiObject>(item: &Item) -> Result<M, DecodeError> {
    decode_object::<M>(item)
}

/// Decode a stream record's `Keys` map as `R`'s primary key.
///
/// `Keys` holds only primary key attributes, so any other attribute is
/// `UnknownAttribute`, unlike `RecordKey::decode_key`, which accepts a full item.
pub fn decode_record_key<R: Record, K: RecordKey<R>>(keys: &Item) -> Result<K, DecodeError> {
    only_keys(R::SCHEMA, keys)?;
    K::decode_key(keys)
}

/// Decode a stream record's `Keys` map as the key of one member of `M`.
///
/// Every key attribute must be present with its declared wire type, nonempty,
/// and within the service size limit; no other attribute may be present, and
/// the key must select a member.
pub fn decode_multi_object_key<M: MultiObject>(keys: &Item) -> Result<MultiKey<M>, DecodeError> {
    let cx = DecodeContext::new(M::SCHEMA.name);
    if let Some(code) = multi_object_fault::<M>() {
        return Err(cx.error(DecodeFault::Custom { code }));
    }
    only_keys(M::SCHEMA, keys)?;
    let primary = M::SCHEMA.primary;
    for (position, max) in [(Some(primary.partition), 2048), (primary.sort, 1024)] {
        let Some(position) = position else { continue };
        let field = &M::SCHEMA.fields[position];
        let cx = cx.at(PathSegment::Attribute(field.name.into()));
        let value = keys
            .get(field.name)
            .ok_or_else(|| cx.error(DecodeFault::Missing))?;
        if wire(value) != Some(field.wire) {
            return Err(crate::value::wrong_type(value, field.wire, &cx));
        }
        crate::record::validate_key(value, max)
            .map_err(|_| cx.error(DecodeFault::KeyFormat(KeyFormatFault::InvalidLength)))?;
    }
    let variant = select_variant::<M>(keys, &cx)?;
    Ok(MultiKey {
        variant,
        item: keys.clone(),
        marker: PhantomData,
    })
}

/// A validated primary key that selects one member of `M`.
pub struct MultiKey<M> {
    variant: usize,
    item: Item,
    marker: PhantomData<fn() -> M>,
}
impl<M: MultiObject> MultiKey<M> {
    /// The member the key selects.
    pub fn variant(&self) -> &'static MultiVariant {
        &M::VARIANTS[self.variant]
    }
    /// Explicit raw access to the key attributes.
    pub fn item(&self) -> &Item {
        &self.item
    }
    /// Decode the key as member `R`'s key, or `None` when it selects another member.
    pub fn member<R: MemberOf<M>, K: RecordKey<R>>(&self) -> Result<Option<K>, DecodeError> {
        const {
            core::mem::forget(<R as MemberOf<M>>::SEAL);
            if let Some(fault) = crate::member_fault::<M, R>() {
                panic!("{}", fault);
            }
        }
        if R::VARIANT != self.variant {
            return Ok(None);
        }
        K::decode_key(&self.item).map(Some)
    }
}
impl<M> Clone for MultiKey<M> {
    fn clone(&self) -> Self {
        Self {
            variant: self.variant,
            item: self.item.clone(),
            marker: PhantomData,
        }
    }
}
impl<M> PartialEq for MultiKey<M> {
    fn eq(&self, other: &Self) -> bool {
        self.variant == other.variant && self.item == other.item
    }
}
impl<M: MultiObject> fmt::Debug for MultiKey<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiKey")
            .field("variant", &self.variant().name)
            .field("item", &"[redacted]")
            .finish()
    }
}

/// How a stream view's images and keys decode: as one record type, or as a
/// multi-object enum. Pick the view's method on [`StreamImages`] to use it.
pub struct StreamDecoder<T, K> {
    schema: RecordSchema,
    image: fn(&Item) -> Result<T, DecodeError>,
    key: fn(&Item) -> Result<K, DecodeError>,
    variant: fn(&Item) -> Option<&'static str>,
}
impl<T, K> Clone for StreamDecoder<T, K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, K> Copy for StreamDecoder<T, K> {}
impl<T, K> fmt::Debug for StreamDecoder<T, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamDecoder")
            .field("record", &self.schema.name)
            .finish_non_exhaustive()
    }
}
impl<R: Record + Document, K: RecordKey<R>> StreamDecoder<R, K> {
    /// Images decode strictly as `R`; keys decode as `K`.
    pub fn record() -> Self {
        Self {
            schema: R::SCHEMA,
            image: decode_record_image::<R>,
            key: decode_record_key::<R, K>,
            variant: |_| None,
        }
    }
}
impl<M: MultiObject> StreamDecoder<M, MultiKey<M>> {
    /// Images decode as members of `M`; keys decode as [`MultiKey<M>`]. A
    /// modification whose images select different members is refused.
    pub fn multi_object() -> Self {
        Self {
            schema: M::SCHEMA,
            image: decode_multi_object_image::<M>,
            key: decode_multi_object_key::<M>,
            variant: |item| {
                select_variant::<M>(item, &DecodeContext::new(M::SCHEMA.name))
                    .ok()
                    .map(|index| M::VARIANTS[index].name)
            },
        }
    }
}

/// A stream record's event name (`eventName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamEventName {
    Insert,
    Modify,
    Remove,
}
impl StreamEventName {
    /// Parse the service's `INSERT`, `MODIFY`, or `REMOVE`; anything else is refused.
    pub fn from_wire(name: &str) -> Result<Self, StreamError> {
        match name {
            "INSERT" => Ok(Self::Insert),
            "MODIFY" => Ok(Self::Modify),
            "REMOVE" => Ok(Self::Remove),
            _ => Err(StreamError::UnknownEventName),
        }
    }
}

/// One part of a stream record, named by errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamImage {
    Keys,
    OldImage,
    NewImage,
}
impl fmt::Display for StreamImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Keys => "Keys",
            Self::OldImage => "OldImage",
            Self::NewImage => "NewImage",
        })
    }
}

/// A change from a `NEW_AND_OLD_IMAGES` stream.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamChange<T> {
    Insert(T),
    Modify { old: T, new: T },
    Remove(T),
}

/// A change from a `NEW_IMAGE` stream: a removal carries only the key.
#[derive(Clone, Debug, PartialEq)]
pub enum NewImageChange<T, K> {
    Insert(T),
    Modify(T),
    Remove(K),
}

/// A change from an `OLD_IMAGE` stream: an insertion carries only the key.
#[derive(Clone, Debug, PartialEq)]
pub enum OldImageChange<T, K> {
    Insert(K),
    Modify(T),
    Remove(T),
}

/// A change from a `KEYS_ONLY` stream.
#[derive(Clone, Debug, PartialEq)]
pub enum KeysOnlyChange<K> {
    Insert(K),
    Modify(K),
    Remove(K),
}

/// The decodable parts of one stream record, borrowed from the caller.
///
/// Each view method requires exactly the images that view carries for the
/// event, refuses any other image, decodes every part strictly, and checks
/// that each image's primary key equals `keys`.
#[derive(Clone, Copy)]
pub struct StreamImages<'a> {
    pub event: StreamEventName,
    pub keys: &'a Item,
    pub old_image: Option<&'a Item>,
    pub new_image: Option<&'a Item>,
}
impl fmt::Debug for StreamImages<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = |image: Option<&Item>| image.map(|_| "[redacted]");
        f.debug_struct("StreamImages")
            .field("event", &self.event)
            .field("keys", &"[redacted]")
            .field("old_image", &redacted(self.old_image))
            .field("new_image", &redacted(self.new_image))
            .finish()
    }
}
impl<'a> StreamImages<'a> {
    /// Decode a `NEW_AND_OLD_IMAGES` record. A modification's images must
    /// select the same member of a multi-object table.
    pub fn new_and_old<T, K>(
        &self,
        decoder: StreamDecoder<T, K>,
    ) -> Result<StreamChange<T>, StreamError> {
        match self.event {
            StreamEventName::Insert => {
                self.absent(StreamImage::OldImage)?;
                let new = self.image(StreamImage::NewImage, decoder)?;
                Ok(StreamChange::Insert(new))
            }
            StreamEventName::Modify => {
                let old_item = self.present(StreamImage::OldImage)?;
                let new_item = self.present(StreamImage::NewImage)?;
                let old = decode(StreamImage::OldImage, old_item, decoder.image)?;
                let new = decode(StreamImage::NewImage, new_item, decoder.image)?;
                let (before, after) = ((decoder.variant)(old_item), (decoder.variant)(new_item));
                if let (Some(old), Some(new)) = (before, after)
                    && old != new
                {
                    return Err(StreamError::VariantMismatch { old, new });
                }
                self.valid_keys(decoder)?;
                self.same_key(StreamImage::OldImage, old_item, decoder.schema)?;
                self.same_key(StreamImage::NewImage, new_item, decoder.schema)?;
                Ok(StreamChange::Modify { old, new })
            }
            StreamEventName::Remove => {
                self.absent(StreamImage::NewImage)?;
                let old = self.image(StreamImage::OldImage, decoder)?;
                Ok(StreamChange::Remove(old))
            }
        }
    }

    /// Decode a `NEW_IMAGE` record.
    pub fn new_image<T, K>(
        &self,
        decoder: StreamDecoder<T, K>,
    ) -> Result<NewImageChange<T, K>, StreamError> {
        self.absent(StreamImage::OldImage)?;
        match self.event {
            StreamEventName::Insert => self
                .image(StreamImage::NewImage, decoder)
                .map(NewImageChange::Insert),
            StreamEventName::Modify => self
                .image(StreamImage::NewImage, decoder)
                .map(NewImageChange::Modify),
            StreamEventName::Remove => {
                self.absent(StreamImage::NewImage)?;
                self.key(decoder).map(NewImageChange::Remove)
            }
        }
    }

    /// Decode an `OLD_IMAGE` record.
    pub fn old_image<T, K>(
        &self,
        decoder: StreamDecoder<T, K>,
    ) -> Result<OldImageChange<T, K>, StreamError> {
        self.absent(StreamImage::NewImage)?;
        match self.event {
            StreamEventName::Insert => {
                self.absent(StreamImage::OldImage)?;
                self.key(decoder).map(OldImageChange::Insert)
            }
            StreamEventName::Modify => self
                .image(StreamImage::OldImage, decoder)
                .map(OldImageChange::Modify),
            StreamEventName::Remove => self
                .image(StreamImage::OldImage, decoder)
                .map(OldImageChange::Remove),
        }
    }

    /// Decode a `KEYS_ONLY` record; any image present is refused.
    pub fn keys_only<T, K>(
        &self,
        decoder: StreamDecoder<T, K>,
    ) -> Result<KeysOnlyChange<K>, StreamError> {
        self.absent(StreamImage::OldImage)?;
        self.absent(StreamImage::NewImage)?;
        let key = self.key(decoder)?;
        Ok(match self.event {
            StreamEventName::Insert => KeysOnlyChange::Insert(key),
            StreamEventName::Modify => KeysOnlyChange::Modify(key),
            StreamEventName::Remove => KeysOnlyChange::Remove(key),
        })
    }

    fn slot(&self, image: StreamImage) -> Option<&'a Item> {
        match image {
            StreamImage::Keys => Some(self.keys),
            StreamImage::OldImage => self.old_image,
            StreamImage::NewImage => self.new_image,
        }
    }
    fn present(&self, image: StreamImage) -> Result<&'a Item, StreamError> {
        self.slot(image).ok_or(StreamError::MissingImage { image })
    }
    fn absent(&self, image: StreamImage) -> Result<(), StreamError> {
        match self.slot(image) {
            Some(_) => Err(StreamError::UnexpectedImage { image }),
            None => Ok(()),
        }
    }
    fn image<T, K>(
        &self,
        image: StreamImage,
        decoder: StreamDecoder<T, K>,
    ) -> Result<T, StreamError> {
        let item = self.present(image)?;
        let value = decode(image, item, decoder.image)?;
        self.valid_keys(decoder)?;
        self.same_key(image, item, decoder.schema)?;
        Ok(value)
    }
    fn key<T, K>(&self, decoder: StreamDecoder<T, K>) -> Result<K, StreamError> {
        decode(StreamImage::Keys, self.keys, decoder.key)
    }
    /// `Keys` decodes with the view's key codec, which refuses any non-key
    /// attribute, even when the images carry the record.
    fn valid_keys<T, K>(&self, decoder: StreamDecoder<T, K>) -> Result<(), StreamError> {
        self.key(decoder).map(drop)
    }
    /// The image's key attributes equal the validated `Keys`. Numbers compare
    /// by value, so equivalent spellings of one number agree.
    fn same_key(
        &self,
        image: StreamImage,
        item: &Item,
        schema: RecordSchema,
    ) -> Result<(), StreamError> {
        let (partition, sort) = key_names(schema).map_err(|source| StreamError::Decode {
            image: StreamImage::Keys,
            source,
        })?;
        for name in [Some(partition), sort].into_iter().flatten() {
            let key = self.keys.get(name).ok_or_else(|| StreamError::Decode {
                image: StreamImage::Keys,
                source: DecodeContext::new(schema.name)
                    .at(PathSegment::Attribute(name.into()))
                    .error(DecodeFault::Missing),
            })?;
            if !item
                .get(name)
                .is_some_and(|value| same_key_value(key, value))
            {
                return Err(StreamError::KeyMismatch {
                    image,
                    attribute: name,
                });
            }
        }
        Ok(())
    }
}

fn same_key_value(a: &AttributeValue, b: &AttributeValue) -> bool {
    match (a, b) {
        (AttributeValue::N(a), AttributeValue::N(b)) => matches!(
            (a.parse::<crate::Decimal>(), b.parse::<crate::Decimal>()),
            (Ok(a), Ok(b)) if a == b
        ),
        _ => a == b,
    }
}

fn decode<T>(
    image: StreamImage,
    item: &Item,
    decode: fn(&Item) -> Result<T, DecodeError>,
) -> Result<T, StreamError> {
    decode(item).map_err(|source| StreamError::Decode { image, source })
}

/// The physical key names; a manual schema whose key positions are out of
/// range is a typed fault, never a panic.
fn key_names(schema: RecordSchema) -> Result<(&'static str, Option<&'static str>), DecodeError> {
    let name = |position: usize| {
        schema.fields.get(position).map(|f| f.name).ok_or_else(|| {
            DecodeContext::new(schema.name).error(DecodeFault::Custom {
                code: "invalid_key_schema",
            })
        })
    };
    Ok((
        name(schema.primary.partition)?,
        schema.primary.sort.map(name).transpose()?,
    ))
}

/// Refuse any attribute of `keys` that is not one of `schema`'s primary keys.
fn only_keys(schema: RecordSchema, keys: &Item) -> Result<(), DecodeError> {
    let (partition, sort) = key_names(schema)?;
    let extra = keys
        .keys()
        .filter(|key| key.as_str() != partition && Some(key.as_str()) != sort)
        .min();
    match extra {
        Some(name) => Err(DecodeContext::new(schema.name)
            .at(PathSegment::Attribute(name.clone()))
            .error(DecodeFault::UnknownAttribute)),
        None => Ok(()),
    }
}

fn wire(value: &AttributeValue) -> Option<DynamoType> {
    match value {
        AttributeValue::S(_) => Some(DynamoType::S),
        AttributeValue::N(_) => Some(DynamoType::N),
        AttributeValue::B(_) => Some(DynamoType::B),
        _ => None,
    }
}

/// A stream record that does not match its view, or an image that fails strict
/// decoding. Display and Debug never include attribute values.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamError {
    /// The event name is not `INSERT`, `MODIFY`, or `REMOVE`.
    UnknownEventName,
    /// The view and event require this image, and it is absent.
    MissingImage { image: StreamImage },
    /// The view or event does not carry this image, and it is present.
    UnexpectedImage { image: StreamImage },
    /// This part failed strict decoding.
    Decode {
        image: StreamImage,
        source: DecodeError,
    },
    /// This image's primary key attribute differs from the record's `Keys`.
    KeyMismatch {
        image: StreamImage,
        attribute: &'static str,
    },
    /// A modification's old and new images select different members.
    VariantMismatch {
        old: &'static str,
        new: &'static str,
    },
}
impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEventName => f.write_str("unknown stream event name"),
            Self::MissingImage { image } => write!(f, "stream record has no {image}"),
            Self::UnexpectedImage { image } => {
                write!(f, "stream record has an unexpected {image}")
            }
            Self::Decode { image, source } => write!(f, "{image}: {source}"),
            Self::KeyMismatch { image, attribute } => {
                write!(f, "{image} key attribute {attribute:?} differs from Keys")
            }
            Self::VariantMismatch { old, new } => {
                write!(f, "OldImage is member {old} but NewImage is member {new}")
            }
        }
    }
}
impl Error for StreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode { source, .. } => Some(source),
            _ => None,
        }
    }
}
