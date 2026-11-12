//! A table binding to a declared secondary index; contains no client.
use std::marker::PhantomData;

use crate::{IndexSchema, Record, Table};

pub struct IndexTable<R: Record, I> {
    pub(crate) table: Table<R>,
    marker: PhantomData<fn(I) -> I>,
}
impl<R: Record> Table<R> {
    pub fn index<I: IndexSchema<R>>(&self, _: I) -> IndexTable<R, I> {
        IndexTable {
            table: self.clone(),
            marker: PhantomData,
        }
    }
}
