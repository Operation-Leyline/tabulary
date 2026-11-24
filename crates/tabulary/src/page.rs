//! Shared one-page outcomes, strict decoding, and response metadata for reads.
use std::fmt;

use aws_sdk_dynamodb::types::ConsumedCapacity;

use crate::cursor::CursorBinding;
use crate::{Cursor, DecodeContext, DecodeError, DecodeFault, Item, PathSegment, ReadDecodeError};

#[derive(Debug)]
pub struct Page<P, C> {
    pub items: Vec<P>,
    pub next: Option<C>,
    pub count: i32,
    pub scanned_count: i32,
    pub capacity: Option<ConsumedCapacity>,
}

/// One `Select::Count` page: service counts and continuation, never items.
#[derive(Debug)]
pub struct CountPage<C> {
    /// Items matching the key condition and filter in this page.
    pub count: u64,
    /// Items evaluated before filtering in this page.
    pub scanned_count: u64,
    pub next_cursor: Option<C>,
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

pub(crate) struct RawPage {
    pub items: Vec<Item>,
    pub metadata: PageMetadata,
    pub capacity: Option<ConsumedCapacity>,
}
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
    pub(crate) fn decode<R, I, P>(
        self,
        binding: &CursorBinding<R, I>,
        decode_item: fn(&Item) -> Result<P, DecodeError>,
    ) -> Result<Page<P, Cursor<R, I>>, Box<ReadDecodeError>> {
        if usize::try_from(self.metadata.count).ok() != Some(self.items.len()) {
            return Err(self.fault(
                0,
                invalid(binding.record_name(), "Count".into(), "invalid_page_count"),
            ));
        }
        if self.metadata.scanned_count < self.metadata.count {
            let source = invalid(
                binding.record_name(),
                "ScannedCount".into(),
                "invalid_scanned_count",
            );
            return Err(self.fault(0, source));
        }
        let next = match self.next(binding) {
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
    fn next<R, I>(
        &self,
        binding: &CursorBinding<R, I>,
    ) -> Result<Option<Cursor<R, I>>, DecodeError> {
        binding
            .cursor(self.metadata.last_evaluated_key.clone())
            .map_err(|_| {
                invalid(
                    binding.record_name(),
                    "LastEvaluatedKey".into(),
                    "invalid_page_cursor",
                )
            })
    }
    /// Decode a `Select::Count` page. Returned items are a protocol fault.
    pub(crate) fn decode_count<R, I>(
        self,
        binding: &CursorBinding<R, I>,
    ) -> Result<CountPage<Cursor<R, I>>, Box<ReadDecodeError>> {
        let Ok(count) = u64::try_from(self.metadata.count) else {
            return Err(self.fault(
                0,
                invalid(binding.record_name(), "Count".into(), "invalid_page_count"),
            ));
        };
        if !self.items.is_empty() {
            return Err(self.fault(
                0,
                invalid(
                    binding.record_name(),
                    "Items".into(),
                    "unexpected_count_items",
                ),
            ));
        }
        let scanned_count = match u64::try_from(self.metadata.scanned_count) {
            Ok(scanned) if scanned >= count => scanned,
            _ => {
                let source = invalid(
                    binding.record_name(),
                    "ScannedCount".into(),
                    "invalid_scanned_count",
                );
                return Err(self.fault(0, source));
            }
        };
        let next_cursor = match self.next(binding) {
            Ok(next) => next,
            Err(source) => return Err(self.fault(0, source)),
        };
        Ok(CountPage {
            count,
            scanned_count,
            next_cursor,
            capacity: self.capacity,
        })
    }
}
fn invalid(record: &'static str, attribute: String, code: &'static str) -> DecodeError {
    DecodeContext::new(record)
        .at(PathSegment::Attribute(attribute))
        .error(DecodeFault::Custom { code })
}

#[cfg(test)]
mod tests;
