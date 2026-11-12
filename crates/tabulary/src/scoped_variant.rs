//! Read-only payload predicates with an inseparable discriminator requirement.
use crate::{
    Attr, BuildError, Condition, Converter, DerivedOnly, FieldDescriptor, Native, PresenceMarker,
    Record, Required, Value,
    condition::{
        self, Contains, Operator, Ordered, Prefix, Size, SizedWire, comparisons,
        ordered_comparisons,
    },
};
use std::marker::PhantomData;

type Invariant<T> = PhantomData<fn(T) -> T>;

/// A payload field selected through one variant. It exposes predicates only;
/// payload writes must replace the group through `Update::transition`.
pub struct ScopedAttr<R, T, C = Native<T>> {
    field: FieldDescriptor,
    group: Result<FieldDescriptor, BuildError>,
    tag: &'static str,
    types: Invariant<(R, T, C)>,
}

#[doc(hidden)]
pub fn scoped<R: Record, G: Value, T, P: PresenceMarker, C: Converter<T>>(
    group: Attr<R, G, Required, DerivedOnly>,
    tag: &'static str,
    name: &'static str,
) -> ScopedAttr<R, T, C> {
    // Flattened payloads share the group's schema position, but keep their own
    // wire name/type; query screening and cursor fingerprints read this position.
    ScopedAttr {
        field: crate::__private::field::<R, T, P, DerivedOnly, C>(name, group.descriptor().index)
            .descriptor(),
        group: group.validate(),
        tag,
        types: PhantomData,
    }
}
impl<R, T, C> Clone for ScopedAttr<R, T, C> {
    fn clone(&self) -> Self {
        Self {
            field: self.field,
            group: self.group.clone(),
            tag: self.tag,
            types: PhantomData,
        }
    }
}
impl<R, T, C> std::fmt::Debug for ScopedAttr<R, T, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScopedAttr([typed])")
    }
}
impl<R: Record, T: 'static, C: Converter<T> + 'static> ScopedAttr<R, T, C> {
    fn guard(self) -> Condition<R> {
        condition::predicate::<R, String, Native<String>>(
            self.group,
            false,
            Operator::Eq,
            vec![self.tag.into()],
        )
    }
    fn predicate(self, operator: Operator, values: Vec<T>) -> Condition<R> {
        condition::predicate::<R, T, C>(Ok(self.field), false, operator, values)
            .guarded(self.guard())
    }
    comparisons!(T; eq => Eq, ne => Ne);
    pub fn exists(self) -> Condition<R> {
        self.predicate(Operator::Exists, vec![])
    }
    pub fn not_exists(self) -> Condition<R> {
        self.predicate(Operator::NotExists, vec![])
    }
    pub fn in_values(self, values: Vec<T>) -> Condition<R> {
        self.predicate(Operator::In, values)
    }
    pub fn eq_path(self, other: Self) -> Condition<R> {
        let fields = (self.field, other.field);
        Condition::new(move || {
            Ok(condition::ConditionNode::Predicate {
                operator: Operator::Eq,
                operands: vec![
                    condition::Operand::Attribute(fields.0),
                    condition::Operand::Attribute(fields.1),
                ],
            })
        })
        .guarded(self.guard())
        .guarded(other.guard())
    }
    pub fn begins_with(self, value: T) -> Condition<R>
    where
        C::Wire: Prefix,
    {
        self.predicate(Operator::BeginsWith, vec![value])
    }
    pub fn contains(self, value: C::Element) -> Condition<R>
    where
        C: Contains<T>,
    {
        condition::predicate::<R, C::Element, C::ElementCodec>(
            Ok(self.field),
            false,
            Operator::Contains,
            vec![value],
        )
        .guarded(self.guard())
    }
    pub fn size(self) -> Size<R>
    where
        C::Wire: SizedWire,
    {
        Size {
            field: Ok(self.field),
            guard: Some(self.guard()),
            owner: PhantomData,
        }
    }
}
impl<R: Record, T: 'static, C: Converter<T> + 'static> ScopedAttr<R, T, C>
where
    C::Wire: Ordered,
{
    ordered_comparisons!(T);
}
