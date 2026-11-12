//! Shared options and success metadata for caller-executed single writes.
use aws_sdk_dynamodb::types::{
    ConsumedCapacity, ItemCollectionMetrics, ReturnItemCollectionMetrics, ReturnValue,
};

/// Full-record return choices for UpdateItem. Partial update returns are excluded.
/// ```compile_fail
/// let _ = tabulary::ReturnItem::UpdatedOld;
/// ```
/// ```compile_fail
/// let _ = tabulary::ReturnItem::UpdatedNew;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReturnItem {
    #[default]
    None,
    AllOld,
    AllNew,
}
impl From<ReturnItem> for ReturnValue {
    fn from(value: ReturnItem) -> Self {
        match value {
            ReturnItem::None => Self::None,
            ReturnItem::AllOld => Self::AllOld,
            ReturnItem::AllNew => Self::AllNew,
        }
    }
}

/// Full-record return choices for PutItem and DeleteItem.
/// ```compile_fail
/// let _ = tabulary::ReturnOld::AllNew;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReturnOld {
    #[default]
    None,
    AllOld,
}
impl From<ReturnOld> for ReturnItem {
    fn from(value: ReturnOld) -> Self {
        match value {
            ReturnOld::None => Self::None,
            ReturnOld::AllOld => Self::AllOld,
        }
    }
}
impl From<ReturnOld> for ReturnValue {
    fn from(value: ReturnOld) -> Self {
        ReturnItem::from(value).into()
    }
}

/// Whether a single write requests item-collection size estimates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollectionMetrics {
    #[default]
    None,
    Size,
}
impl From<CollectionMetrics> for ReturnItemCollectionMetrics {
    fn from(value: CollectionMetrics) -> Self {
        match value {
            CollectionMetrics::None => Self::None,
            CollectionMetrics::Size => Self::Size,
        }
    }
}

/// A committed single write's optional full record and unmodified SDK accounting.
#[derive(Debug)]
pub struct WriteResult<R> {
    pub item: Option<R>,
    pub capacity: Option<ConsumedCapacity>,
    pub item_collection_metrics: Option<ItemCollectionMetrics>,
}

#[cfg(test)]
mod tests {
    use aws_sdk_dynamodb::types::{ReturnItemCollectionMetrics, ReturnValue};

    use crate::{CollectionMetrics, ReturnItem, ReturnOld};

    #[test]
    fn full_record_return_choices_map_to_exact_sdk_values() {
        for (choice, expected) in [
            (ReturnItem::None, ReturnValue::None),
            (ReturnItem::AllOld, ReturnValue::AllOld),
            (ReturnItem::AllNew, ReturnValue::AllNew),
        ] {
            assert_eq!(ReturnValue::from(choice), expected);
        }
        for (choice, expected) in [
            (ReturnOld::None, ReturnItem::None),
            (ReturnOld::AllOld, ReturnItem::AllOld),
        ] {
            assert_eq!(ReturnItem::from(choice), expected);
            assert_eq!(ReturnValue::from(choice), ReturnValue::from(expected));
        }
        assert_eq!(ReturnItem::default(), ReturnItem::None);
        assert_eq!(ReturnOld::default(), ReturnOld::None);
    }

    #[test]
    fn collection_metrics_are_an_explicit_request_option() {
        assert_eq!(CollectionMetrics::default(), CollectionMetrics::None);
        assert_eq!(
            ReturnItemCollectionMetrics::from(CollectionMetrics::None),
            ReturnItemCollectionMetrics::None
        );
        assert_eq!(
            ReturnItemCollectionMetrics::from(CollectionMetrics::Size),
            ReturnItemCollectionMetrics::Size
        );
    }
}
