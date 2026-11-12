//! Shared one-page outcomes, strict decoding, and response metadata for reads.
use std::fmt;

use aws_sdk_dynamodb::types::ConsumedCapacity;

use crate::cursor::CursorBinding;
use crate::{
    Cursor, DecodeContext, DecodeError, DecodeFault, Item, PathSegment, ReadDecodeError, Record,
};

#[derive(Debug)]
pub struct Page<P, C> {
    pub items: Vec<P>,
    pub next: Option<C>,
    pub count: i32,
    pub scanned_count: i32,
    pub capacity: Option<ConsumedCapacity>,
}

/// Unmodified SDK page accounting, retained even when a row cannot be decoded.
pub struct PageMetadata {
    pub count: i32,
    pub scanned_count: i32,
    pub(crate) last_evaluated_key: Option<Item>,
}
impl PageMetadata {
    /// Explicit raw access; the service continuation may contain sensitive keys.
    pub fn last_evaluated_key(&self) -> Option<&Item> {
        self.last_evaluated_key.as_ref()
    }
}
impl fmt::Debug for PageMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageMetadata")
            .field("count", &self.count)
            .field("scanned_count", &self.scanned_count)
            .field("last_evaluated_key", &"[redacted]")
            .finish()
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Used by the stacked Query adapter")
)]
pub(crate) struct RawPage {
    pub items: Vec<Item>,
    pub metadata: PageMetadata,
    pub capacity: Option<ConsumedCapacity>,
}
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Used by the stacked Query adapter")
)]
impl RawPage {
    fn fault(self, position: usize, source: DecodeError) -> Box<ReadDecodeError> {
        Box::new(ReadDecodeError {
            position,
            source,
            capacity: self.capacity,
            page: Some(self.metadata),
            raw_items: self.items,
        })
    }
    pub(crate) fn decode<R: Record, I, P>(
        self,
        binding: &CursorBinding<R, I>,
        decode_item: fn(&Item) -> Result<P, DecodeError>,
    ) -> Result<Page<P, Cursor<R, I>>, Box<ReadDecodeError>> {
        let invalid = |attribute, code| {
            DecodeContext::new(R::SCHEMA.name)
                .at(PathSegment::Attribute(attribute))
                .error(DecodeFault::Custom { code })
        };
        if usize::try_from(self.metadata.count).ok() != Some(self.items.len()) {
            return Err(self.fault(0, invalid("Count".into(), "invalid_page_count")));
        }
        if self.metadata.scanned_count < self.metadata.count {
            return Err(self.fault(0, invalid("ScannedCount".into(), "invalid_scanned_count")));
        }
        let next = binding
            .cursor(self.metadata.last_evaluated_key.clone())
            .map_err(|_| invalid("LastEvaluatedKey".into(), "invalid_page_cursor"));
        let next = match next {
            Ok(next) => next,
            Err(source) => return Err(self.fault(0, source)),
        };
        let mut items = Vec::with_capacity(self.items.len());
        for (position, raw) in self.items.iter().enumerate() {
            match decode_item(raw) {
                Ok(item) => items.push(item),
                Err(source) => return Err(self.fault(position, source)),
            }
        }
        Ok(Page {
            items,
            next,
            count: self.metadata.count,
            scanned_count: self.metadata.scanned_count,
            capacity: self.capacity,
        })
    }
}

#[cfg(test)]
mod tests;
