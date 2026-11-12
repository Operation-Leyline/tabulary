//! One inspectable transaction request; execution belongs to the caller.
use std::fmt;

use crate::{BilledSize, BuildError, Capacity, CollectionMetrics, Failure, Item, TxAction};
use aws_sdk_dynamodb::{
    operation::transact_write_items::{
        TransactWriteItemsInput, builders::TransactWriteItemsFluentBuilder,
    },
    types::{CancellationReason, TransactWriteItem},
};

fn constraint(code: &'static str) -> BuildError {
    BuildError::OperationConstraint { code }
}

mod sealed {
    use super::*;
    pub trait Actions {
        fn append(&self, items: &mut Vec<TransactWriteItem>, targets: &mut Vec<(String, Item)>);
    }
}
/// Frozen actions, recursive tuples (arity 1..=12), and homogeneous action vectors.
///
/// Requests flatten depth-first, left-to-right; `Reasons` retains the original shape.
/// Empty vectors contribute no actions, but the final request must have 1..=100.
pub trait TxActions: sealed::Actions {
    type Reasons;
    #[doc(hidden)]
    fn decode_reasons(
        &self,
        reasons: &mut std::slice::Iter<'_, CancellationReason>,
    ) -> Self::Reasons;
}
impl<R> sealed::Actions for TxAction<R> {
    fn append(&self, items: &mut Vec<TransactWriteItem>, targets: &mut Vec<(String, Item)>) {
        items.push(self.input().clone());
        targets.push(self.target.clone());
    }
}
impl<R> TxActions for TxAction<R> {
    type Reasons = Failure<R>;
    fn decode_reasons(
        &self,
        reasons: &mut std::slice::Iter<'_, CancellationReason>,
    ) -> Self::Reasons {
        self.decode_reason(reasons.next())
    }
}
impl<R> sealed::Actions for Vec<TxAction<R>> {
    fn append(&self, items: &mut Vec<TransactWriteItem>, targets: &mut Vec<(String, Item)>) {
        for action in self {
            action.append(items, targets);
        }
    }
}
impl<R> TxActions for Vec<TxAction<R>> {
    type Reasons = Vec<Failure<R>>;
    fn decode_reasons(
        &self,
        reasons: &mut std::slice::Iter<'_, CancellationReason>,
    ) -> Self::Reasons {
        self.iter()
            .map(|action| action.decode_reasons(reasons))
            .collect()
    }
}
macro_rules! tuples {
    ($($t:ident : $i:tt),+) => {
        impl<$($t: TxActions),+> sealed::Actions for ($($t,)+) {
            fn append(&self, items: &mut Vec<TransactWriteItem>, targets: &mut Vec<(String, Item)>) {
                $(self.$i.append(items, targets);)+
            }
        }
        impl<$($t: TxActions),+> TxActions for ($($t,)+) {
            type Reasons = ($($t::Reasons,)+);
            fn decode_reasons(&self, reasons: &mut std::slice::Iter<'_, CancellationReason>) -> Self::Reasons {
                ($(self.$i.decode_reasons(reasons),)+)
            }
        }
    };
}
tuples!(A:0);
tuples!(A:0, B:1);
tuples!(A:0, B:1, C:2);
tuples!(A:0, B:1, C:2, D:3);
tuples!(A:0, B:1, C:2, D:3, E:4);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
tuples!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

pub struct TransactionBuilder<A> {
    actions: A,
    token: Option<String>,
    capacity: Capacity,
    metrics: CollectionMetrics,
}
pub fn transaction<A: TxActions>(actions: A) -> TransactionBuilder<A> {
    TransactionBuilder {
        actions,
        token: None,
        capacity: Capacity::None,
        metrics: CollectionMetrics::None,
    }
}
impl<A: TxActions> TransactionBuilder<A> {
    pub fn client_request_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn collection_metrics(mut self, metrics: CollectionMetrics) -> Self {
        self.metrics = metrics;
        self
    }
    pub fn build(self) -> Result<PreparedTransaction<A>, BuildError> {
        if self
            .token
            .as_ref()
            .is_some_and(|t| !(1..=36).contains(&t.chars().count()))
        {
            return Err(constraint("invalid_client_request_token"));
        }
        let (mut items, mut targets) = (Vec::new(), Vec::new());
        self.actions.append(&mut items, &mut targets);
        let mut size = validate(&items, &targets)?;
        size.conservative_wire_bytes =
            crate::request_size::transaction(&items, self.token.as_deref())?;
        let input = TransactWriteItemsInput::builder()
            .set_transact_items(Some(items))
            .set_client_request_token(self.token)
            .set_return_consumed_capacity(self.capacity.sdk())
            .return_item_collection_metrics(self.metrics.into())
            .build()
            .map_err(|_| constraint("invalid_transaction_input"))?;
        Ok(PreparedTransaction {
            input,
            actions: self.actions,
            size,
        })
    }
}

/// One immutable request with its original typed action shape; never auto-split.
pub struct PreparedTransaction<A> {
    input: TransactWriteItemsInput,
    actions: A,
    size: TransactionSize,
}
/// Local size evidence; unknown stored rows are never estimated from their keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionSize {
    /// Conservative put-item contribution toward the 4 MiB aggregate item cap.
    pub known_put_item_bytes: usize,
    /// Delete, update and condition-check rows whose stored sizes are unknown.
    pub unknown_item_count: usize,
    /// Conservative JSON request bytes, including bindings and base64 expansion.
    /// This is separate from the aggregate item cap and has no local 4 MiB gate.
    pub conservative_wire_bytes: usize,
}
impl<A> PreparedTransaction<A> {
    pub fn size(&self) -> TransactionSize {
        self.size
    }
    pub fn input(&self) -> &TransactWriteItemsInput {
        &self.input
    }
    pub fn actions(&self) -> &A {
        &self.actions
    }
    /// Replace every modeled request field, including absent caller token/capacity.
    pub fn apply(
        &self,
        builder: TransactWriteItemsFluentBuilder,
    ) -> TransactWriteItemsFluentBuilder {
        builder
            .set_transact_items(self.input.transact_items.clone())
            .set_client_request_token(self.input.client_request_token.clone())
            .set_return_consumed_capacity(self.input.return_consumed_capacity.clone())
            .set_return_item_collection_metrics(self.input.return_item_collection_metrics.clone())
    }
}
impl<A> fmt::Debug for PreparedTransaction<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreparedTransaction([redacted])")
    }
}

fn table_identity(table: &str) -> (Option<(&str, &str, &str)>, &str) {
    let parts: Vec<_> = table.splitn(6, ':').collect();
    if let ["arn", partition, "dynamodb", region, account, resource] = parts.as_slice()
        && let Some(name) = resource.strip_prefix("table/")
    {
        return (Some((partition, region, account)), name);
    }
    (None, table)
}
fn validate(
    items: &[TransactWriteItem],
    targets: &[(String, Item)],
) -> Result<TransactionSize, BuildError> {
    if items.is_empty() {
        return Err(BuildError::EmptyOperation);
    }
    if items.len() > 100 {
        return Err(BuildError::RequestLimit {
            max: 100,
            actual: items.len(),
        });
    }
    let mut location = None;
    for (i, (table, key)) in targets.iter().enumerate() {
        let (scope, name) = table_identity(table);
        if let Some(scope) = scope {
            if location.is_some_and(|previous| previous != scope) {
                return Err(constraint("transaction_table_location"));
            }
            location = Some(scope);
        }
        if targets[..i]
            .iter()
            .any(|(t, k)| table_identity(t).1 == name && k == key)
        {
            return Err(constraint("duplicate_transaction_target"));
        }
    }
    let mut known = 0usize;
    for put in items.iter().filter_map(|item| item.put.as_ref()) {
        let size = BilledSize::of_item(&put.item)?;
        size.ensure_at_most(400 * 1024)?;
        known = known
            .checked_add(size.conservative_item_bytes())
            .ok_or(BuildError::SizeOverflow)?;
    }
    if known > 4 * 1024 * 1024 {
        return Err(BuildError::RequestLimit {
            max: 4 * 1024 * 1024,
            actual: known,
        });
    }
    Ok(TransactionSize {
        known_put_item_bytes: known,
        unknown_item_count: items.iter().filter(|item| item.put.is_none()).count(),
        conservative_wire_bytes: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_dynamodb::types::{AttributeValue as Av, Put};

    #[test]
    fn final_count_is_nonempty_and_capped_without_splitting() {
        // Exercise the final flattened gate directly, without splitting actions.
        for count in [0, 1, 100, 101] {
            let items = vec![TransactWriteItem::builder().build(); count];
            let expected = match count {
                0 => Err(BuildError::EmptyOperation),
                101 => Err(BuildError::RequestLimit {
                    max: 100,
                    actual: 101,
                }),
                _ => Ok(()),
            };
            assert_eq!(validate(&items, &[]).map(|_| ()), expected);
            assert_eq!(items.len(), count);
        }
    }

    #[test]
    fn known_put_bytes_are_bounded_without_claiming_existing_item_sizes() {
        let put = |bytes| {
            TransactWriteItem::builder()
                .put(
                    Put::builder()
                        .table_name("orders")
                        .item("payload", Av::S("x".repeat(bytes)))
                        .build()
                        .unwrap(),
                )
                .build()
        };
        let small = put(380 * 1024);
        assert!(validate(std::slice::from_ref(&small), &[]).is_ok());
        assert!(matches!(
            validate(&vec![small; 12], &[]),
            Err(BuildError::RequestLimit { max: 4_194_304, .. })
        ));
        assert!(matches!(
            validate(&[put(400 * 1024)], &[]),
            Err(BuildError::RequestLimit { max: 409_600, .. })
        ));
        let mut boundary = vec![put(400 * 1024 - 7); 10];
        boundary.push(put(4 * 1024 * 1024 - 10 * 400 * 1024 - 7));
        assert_eq!(
            validate(&boundary, &[]).unwrap().known_put_item_bytes,
            4_194_304
        );
        boundary[10] = put(4 * 1024 * 1024 - 10 * 400 * 1024 - 6);
        assert_eq!(
            validate(&boundary, &[]),
            Err(BuildError::RequestLimit {
                max: 4_194_304,
                actual: 4_194_305
            })
        );
    }
}
