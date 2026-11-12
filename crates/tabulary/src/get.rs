//! GetItem preparation and decoding; the caller alone sends the SDK request.
use std::marker::PhantomData;

use aws_sdk_dynamodb::{
    error::SdkError,
    operation::get_item::{
        GetItemError, GetItemInput, GetItemOutput, builders::GetItemFluentBuilder,
    },
};

use crate::{
    BuildError, Capacity, DecodeContext, DecodeError, Document, GetResult, Item, PreparedRequest,
    Projection, ReadConsistency, ReadDecodeError, ReadError, Record, RecordKey,
    RenderedExpressions, Table,
};

pub type PreparedGet<P> = PreparedRequest<GetItemInput, P>;
type Selection = fn() -> Result<Vec<&'static str>, BuildError>;

/// Owns its key and table binding. All fallible encoding happens at build.
pub struct GetBuilder<R, K, P = R> {
    table: String,
    key: K,
    consistency: ReadConsistency,
    capacity: Capacity,
    selection: Option<Selection>,
    decode_item: fn(&Item) -> Result<P, DecodeError>,
    marker: PhantomData<fn() -> R>,
}
impl<R: Record + Document> Table<R> {
    pub fn get<K: RecordKey<R>>(&self, key: K, consistency: ReadConsistency) -> GetBuilder<R, K> {
        GetBuilder {
            table: self.name.clone(),
            key,
            consistency,
            capacity: Capacity::None,
            selection: None,
            decode_item: |item| R::decode_map(item, &mut DecodeContext::new(R::NAME)),
            marker: PhantomData,
        }
    }
}
impl<R: Record, K: RecordKey<R>, P> GetBuilder<R, K, P> {
    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = capacity;
        self
    }
    pub fn select<V: Projection<Source = R>>(self) -> GetBuilder<R, K, V> {
        GetBuilder {
            table: self.table,
            key: self.key,
            consistency: self.consistency,
            capacity: self.capacity,
            selection: Some(V::attributes),
            decode_item: V::from_item,
            marker: PhantomData,
        }
    }
    pub fn build(self) -> Result<PreparedGet<P>, BuildError> {
        let key = self.key.encode_key()?;
        let mut expressions = RenderedExpressions::default();
        if let Some(select) = self.selection {
            let attributes = select()?;
            if attributes.is_empty() {
                return Err(BuildError::EmptyOperation);
            }
            expressions.projection = Some(
                attributes
                    .iter()
                    .map(|name| expressions.aliases.name(name))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        let names = (!expressions.names().is_empty()).then(|| expressions.names().clone());
        let input = GetItemInput::builder()
            .table_name(self.table)
            .set_key(Some(key))
            .consistent_read(self.consistency == ReadConsistency::Strong)
            .set_return_consumed_capacity(self.capacity.sdk())
            .set_projection_expression(expressions.projection.clone())
            .set_expression_attribute_names(names)
            .build()
            .map_err(|_| BuildError::OperationConstraint {
                code: "invalid_get_input",
            })?;
        Ok(PreparedRequest {
            input,
            expressions,
            decode_item: self.decode_item,
        })
    }
}
impl<P> PreparedGet<P> {
    /// Replace every modeled input field, including absent legacy fields.
    /// Transport configuration on the supplied fluent builder remains unchanged.
    pub fn apply(&self, builder: GetItemFluentBuilder) -> GetItemFluentBuilder {
        let input = &self.input;
        builder
            .set_table_name(input.table_name.clone())
            .set_key(input.key.clone())
            .set_attributes_to_get(input.attributes_to_get.clone())
            .set_consistent_read(input.consistent_read)
            .set_return_consumed_capacity(input.return_consumed_capacity.clone())
            .set_projection_expression(input.projection_expression.clone())
            .set_expression_attribute_names(input.expression_attribute_names.clone())
    }
    pub fn decode(
        &self,
        result: Result<GetItemOutput, SdkError<GetItemError>>,
    ) -> Result<GetResult<P>, ReadError<GetItemError>> {
        let output = result.map_err(|error| ReadError::Sdk(Box::new(error)))?;
        let capacity = output.consumed_capacity;
        let item = match output.item {
            None => None,
            Some(raw) => match (self.decode_item)(&raw) {
                Ok(item) => Some(item),
                Err(source) => {
                    return Err(ReadError::Decode(Box::new(ReadDecodeError {
                        position: 0,
                        source,
                        capacity,
                        raw_items: vec![raw],
                    })));
                }
            },
        };
        Ok(GetResult { item, capacity })
    }
}
