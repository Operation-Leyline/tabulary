//! Runtime table binding, independent of SDK clients and deployment.
use std::{fmt, marker::PhantomData};

use crate::{BuildError, Record};

/// A record schema bound to an owned table name or ARN. Contains no client.
pub struct Table<R: Record> {
    pub(crate) name: String,
    marker: PhantomData<fn() -> R>,
}
impl<R: Record> Table<R> {
    /// Reject an empty identifier; preserve all other names and ARNs verbatim.
    pub fn new(name: impl Into<String>) -> Result<Self, BuildError> {
        let name = name.into();
        if name.is_empty() {
            return Err(BuildError::OperationConstraint {
                code: "empty_table_name",
            });
        }
        Ok(Self {
            name,
            marker: PhantomData,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}
impl<R: Record> Clone for Table<R> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            marker: PhantomData,
        }
    }
}
impl<R: Record> fmt::Debug for Table<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table")
            .field("record", &R::SCHEMA.name)
            .finish_non_exhaustive()
    }
}
