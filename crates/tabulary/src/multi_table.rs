//! A table binding that admits only the declared members of one closed enum.
use std::{fmt, marker::PhantomData, num::NonZeroU32};

use aws_sdk_dynamodb::operation::{query::QueryInput, scan::ScanInput};

use crate::condition::{ConditionNode, Operand};
use crate::cursor::CursorBinding;
use crate::multi_object::{decode_object, member_fault, multi_object_fault};
use crate::{
    BatchAction, BatchBudget, BatchGet, BatchWriteBatch, BuildError, Capacity, CheckBuilder,
    Condition, Cursor, DeleteBuilder, FieldDescriptor, GetBuilder, KeyCondition, KeySchema,
    MemberOf, MultiObject, PreparedQuery, PreparedRequest, PreparedScan, PrimarySchema, PutBuilder,
    QueryBuilder, QueryOrder, ReadConsistency, Record, RecordKey, RenderedExpressions, ScanSegment,
    Table, UpdateBuilder,
};

/// Cursor marker for a partition query decoded as every member of a table.
pub struct MultiPartition;

/// One table holding the closed set of record types declared by `M`.
///
/// Every typed operation names a record `R: MemberOf<M>`; any other record type
/// fails to compile. Reads that may return several members decode as `M`.
/// Like [`Table`], the binding holds a name and never a client.
pub struct MultiTable<M> {
    name: String,
    marker: PhantomData<fn() -> M>,
}

impl<M: MultiObject> MultiTable<M> {
    /// Reject an empty identifier and an invalid manual multi-object schema.
    pub fn new(name: impl Into<String>) -> Result<Self, BuildError> {
        let name = name.into();
        if name.is_empty() {
            return Err(BuildError::OperationConstraint {
                code: "empty_table_name",
            });
        }
        if let Some(code) = multi_object_fault::<M>() {
            return Err(BuildError::OperationConstraint { code });
        }
        Ok(Self {
            name,
            marker: PhantomData,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The ordinary single-record binding for one member. Its operations are
    /// the plain `Table<R>` ones; `MultiTable::update` adds an existence guard.
    #[cfg_attr(
        feature = "derive",
        doc = r#"
Every member operation goes through this method, which evaluates `R::SEAL` and
`member_fault` at compile time. A seal that diverges instead of being
constructed fails when the operation is monomorphized, in any build:

```compile_fail
mod accounts {
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
    pub struct Customer { pub customer_ref: String }

    #[derive(tabulary::MultiObject)]
    #[tabulary(partition = "pk")]
    pub enum AccountTable {
        #[tabulary(partition(prefix = "CUST#"))]
        Customer(Customer),
    }
}
use accounts::AccountTable;
use tabulary::{MemberOf, MultiObject, MultiTable};

// Same record name, keys and prefix as the real member, so only the seal differs.
mod impostor {
    #[derive(tabulary::Record)]
    #[tabulary(rename_all = "snake_case", partition(name = "pk", template = "CUST#{customer_ref:text}"))]
    pub struct Customer { pub customer_ref: String }
}

impl MemberOf<AccountTable> for impostor::Customer {
    const VARIANT: usize = 0;
    const SEAL: <AccountTable as MultiObject>::MemberSeal<Self> = panic!("no seal");
    fn into_object(self) -> AccountTable {
        AccountTable::Customer(accounts::Customer { customer_ref: self.customer_ref })
    }
}

let accounts = MultiTable::<AccountTable>::new("accounts").unwrap();
let _ = accounts.member::<impostor::Customer>();
```
"#
    )]
    pub fn member<R: MemberOf<M>>(&self) -> Table<R> {
        // Evaluate the seal: an associated const is otherwise checked only on use,
        // so a diverging `SEAL` would let an undeclared type through.
        const {
            core::mem::forget(<R as MemberOf<M>>::SEAL);
            if let Some(fault) = member_fault::<M, R>() {
                panic!("{}", fault);
            }
        }
        Table::bound(&self.name)
    }
    pub fn put<'a, R: MemberOf<M>>(&self, record: &'a R) -> PutBuilder<'a, R> {
        self.member::<R>().put(record)
    }
    pub fn get<R: MemberOf<M>, K: RecordKey<R>>(
        &self,
        key: K,
        consistency: ReadConsistency,
    ) -> GetBuilder<R, K> {
        self.member::<R>().get(key, consistency)
    }
    /// Update an existing member only. The builder starts with an
    /// `attribute_exists` guard on the partition key, which later conditions
    /// are ANDed with and transaction conversion keeps. Without it, an update
    /// of an absent key would create a partial item that the member's strict
    /// decoder, and therefore every polymorphic page holding it, would refuse.
    /// Create members with a full-record `put`.
    pub fn update<R: MemberOf<M>, K: RecordKey<R>>(&self, key: K) -> UpdateBuilder<R, K> {
        self.member::<R>().update(key).condition(exists::<R>())
    }
    pub fn delete<R: MemberOf<M>, K: RecordKey<R>>(&self, key: K) -> DeleteBuilder<R, K> {
        self.member::<R>().delete(key)
    }
    /// A transaction-only condition check on one member.
    pub fn check<R: MemberOf<M>, K: RecordKey<R>>(
        &self,
        key: K,
        condition: Condition<R>,
    ) -> CheckBuilder<R, K> {
        self.member::<R>().check(key, condition)
    }
    pub fn batch_get<R: MemberOf<M>, K: RecordKey<R>>(
        &self,
        keys: Vec<K>,
        consistency: ReadConsistency,
        budget: BatchBudget,
    ) -> Result<BatchGet<R, K>, BuildError> {
        self.member::<R>().batch_get(keys, consistency, budget)
    }
    pub fn batch_write<R: MemberOf<M>>(
        &self,
        actions: Vec<BatchAction<R>>,
        budget: BatchBudget,
    ) -> Result<BatchWriteBatch<R>, BuildError> {
        self.member::<R>().batch_write(actions, budget)
    }

    /// Query one member only. An open sort slot is narrowed to the member's
    /// sort discriminator (`begins_with` for a prefix, equality for an exact
    /// literal), so other members sharing the partition are never returned.
    /// Every returned item is still decoded strictly as `R`.
    pub fn query_entity<R: MemberOf<M>, I: PrimarySchema<R>>(
        &self,
        key: KeyCondition<R, I>,
        consistency: ReadConsistency,
    ) -> QueryBuilder<R, I, <I as KeySchema<R>>::Sort> {
        let pattern = M::VARIANTS[<R as MemberOf<M>>::VARIANT].keys.sort;
        self.member::<R>()
            .query(key.restrict_sort(pattern), consistency)
    }

    /// Query a partition and decode every item as whichever member its keys select.
    ///
    /// The key condition is built from any member's primary schema, since all
    /// members share the physical keys. Items keep service order; an item that
    /// matches no member, or fails its member's strict decode, fails the page.
    pub fn query<R: MemberOf<M>, I: PrimarySchema<R>, S>(
        &self,
        key: KeyCondition<R, I, S>,
        consistency: ReadConsistency,
    ) -> MultiQueryBuilder<M, R, I, S> {
        let _ = self.member::<R>();
        MultiQueryBuilder {
            table: self.name.clone(),
            key,
            consistency,
            capacity: Capacity::None,
            order: QueryOrder::Ascending,
            limit: None,
            resume: None,
            marker: PhantomData,
        }
    }

    /// Scan the table and decode every item as whichever member its keys select.
    pub fn scan(&self, consistency: ReadConsistency) -> MultiScanBuilder<M> {
        MultiScanBuilder {
            table: self.name.clone(),
            consistency,
            capacity: Capacity::None,
            limit: None,
            segment: None,
            resume: None,
        }
    }
}
impl<M> Clone for MultiTable<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            marker: PhantomData,
        }
    }
}
impl<M: MultiObject> fmt::Debug for MultiTable<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiTable")
            .field("object", &M::SCHEMA.name)
            .finish_non_exhaustive()
    }
}

fn exists<R: Record>() -> Condition<R> {
    Condition::new(|| {
        let position = R::SCHEMA.primary.partition;
        let field = *R::SCHEMA
            .fields
            .get(position)
            .ok_or(BuildError::OperationConstraint {
                code: "invalid_key_schema",
            })?;
        Ok(ConditionNode::Predicate {
            operator: crate::condition::Operator::Exists,
            operands: vec![Operand::Attribute(field)],
        })
    })
}

fn limit(limit: Option<NonZeroU32>) -> Result<Option<i32>, BuildError> {
    limit
        .map(|limit| {
            i32::try_from(limit.get()).map_err(|_| BuildError::RequestLimit {
                max: i32::MAX as usize,
                actual: limit.get() as usize,
            })
        })
        .transpose()
}

// Rebind member key descriptors to the shared keys-only schema, so the cursor
// identity is the same whichever member built the key condition.
fn rebind(node: &ConditionNode, fields: &'static [FieldDescriptor]) -> ConditionNode {
    let field = |f: &FieldDescriptor| *fields.iter().find(|k| k.name == f.name).unwrap_or(f);
    match node {
        ConditionNode::Predicate { operator, operands } => ConditionNode::Predicate {
            operator: *operator,
            operands: operands
                .iter()
                .map(|operand| match operand {
                    Operand::Attribute(f) => Operand::Attribute(field(f)),
                    Operand::Size(f) => Operand::Size(field(f)),
                    // Key conditions name top-level keys only; paths and values pass through.
                    Operand::Path(_) | Operand::PathSize(_) | Operand::Value(_) => operand.clone(),
                })
                .collect(),
        },
        ConditionNode::And(a, b) => {
            ConditionNode::And(Box::new(rebind(a, fields)), Box::new(rebind(b, fields)))
        }
        ConditionNode::Or(a, b) => {
            ConditionNode::Or(Box::new(rebind(a, fields)), Box::new(rebind(b, fields)))
        }
        ConditionNode::Not(a) => ConditionNode::Not(Box::new(rebind(a, fields))),
    }
}

/// A polymorphic partition query. It has no filter or projection: both are
/// defined per record, while this page may hold every member.
pub struct MultiQueryBuilder<M, R: Record, I: KeySchema<R>, S> {
    table: String,
    key: KeyCondition<R, I, S>,
    consistency: ReadConsistency,
    capacity: Capacity,
    order: QueryOrder,
    limit: Option<NonZeroU32>,
    resume: Option<Cursor<M, MultiPartition>>,
    marker: PhantomData<fn() -> M>,
}
impl<M: MultiObject, R: Record, I: KeySchema<R>, S> MultiQueryBuilder<M, R, I, S> {
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn order(mut self, order: QueryOrder) -> Self {
        self.order = order;
        self
    }
    /// Evaluated items per service call; never a matching-row goal.
    pub fn limit(mut self, limit: NonZeroU32) -> Self {
        self.limit = Some(limit);
        self
    }
    pub fn resume(mut self, cursor: Cursor<M, MultiPartition>) -> Self {
        self.resume = Some(cursor);
        self
    }
    pub fn build(self) -> Result<PreparedQuery<M, MultiPartition, M>, BuildError> {
        let limit = limit(self.limit)?;
        let key = self.key.build()?;
        let root = rebind(key.root(), M::SCHEMA.fields);
        let binding = CursorBinding::multi(
            &self.table,
            M::SCHEMA,
            Some((&root, self.order)),
            None,
            self.consistency,
        )?;
        let start = self
            .resume
            .as_ref()
            .map(|cursor| binding.resume(cursor).cloned())
            .transpose()?;
        let expressions = key.render();
        expressions.validate()?;
        let input = QueryInput::builder()
            .table_name(&self.table)
            .consistent_read(self.consistency == ReadConsistency::Strong)
            .scan_index_forward(self.order == QueryOrder::Ascending)
            .set_limit(limit)
            .set_exclusive_start_key(start)
            .set_return_consumed_capacity(self.capacity.sdk())
            .set_key_condition_expression(expressions.key_condition.clone())
            .set_expression_attribute_names(Some(expressions.names().clone()))
            .set_expression_attribute_values(Some(expressions.values().clone()))
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_query_input",
            })?;
        Ok(PreparedQuery {
            request: prepared(input, expressions),
            binding,
        })
    }
}
impl<M, R: Record, I: KeySchema<R>, S> fmt::Debug for MultiQueryBuilder<M, R, I, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MultiQueryBuilder([unbuilt])")
    }
}

/// A polymorphic scan of the whole table, optionally one parallel segment.
pub struct MultiScanBuilder<M> {
    table: String,
    consistency: ReadConsistency,
    capacity: Capacity,
    limit: Option<NonZeroU32>,
    segment: Option<ScanSegment>,
    resume: Option<Cursor<M, ()>>,
}
impl<M: MultiObject> MultiScanBuilder<M> {
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    /// Evaluated items per service call.
    pub fn limit(mut self, limit: NonZeroU32) -> Self {
        self.limit = Some(limit);
        self
    }
    pub fn segment(mut self, segment: ScanSegment) -> Self {
        self.segment = Some(segment);
        self
    }
    pub fn resume(mut self, cursor: Cursor<M, ()>) -> Self {
        self.resume = Some(cursor);
        self
    }
    pub fn build(self) -> Result<PreparedScan<M, (), M>, BuildError> {
        let limit = limit(self.limit)?;
        let segment = self.segment.map(|s| (s.segment, s.total));
        let binding =
            CursorBinding::multi(&self.table, M::SCHEMA, None, segment, self.consistency)?;
        let start = self
            .resume
            .as_ref()
            .map(|cursor| binding.resume(cursor).cloned())
            .transpose()?;
        let input = ScanInput::builder()
            .table_name(&self.table)
            .consistent_read(self.consistency == ReadConsistency::Strong)
            .set_limit(limit)
            .set_segment(segment.map(|(segment, _)| segment as i32))
            .set_total_segments(segment.map(|(_, total)| total as i32))
            .set_exclusive_start_key(start)
            .set_return_consumed_capacity(self.capacity.sdk())
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_scan_input",
            })?;
        Ok(PreparedScan {
            request: prepared(input, RenderedExpressions::default()),
            binding,
        })
    }
}
impl<M> fmt::Debug for MultiScanBuilder<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MultiScanBuilder([unbuilt])")
    }
}

fn prepared<T, M: MultiObject>(
    input: T,
    expressions: RenderedExpressions,
) -> PreparedRequest<T, M> {
    PreparedRequest {
        input,
        expressions,
        decode_item: decode_object::<M>,
    }
}
