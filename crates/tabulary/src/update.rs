//! Owned update actions and typed SET operands, before textual rendering.
use std::{fmt, marker::PhantomData};

use crate::{
    AccessMarker, Attr, BuildError, Converter, EncodeContext, FieldDescriptor, List, Mutable,
    Native, Optional, PathSegment, PresenceMarker, Record, Value, WireType, condition::Operand,
    wire,
};

/// Wire categories supporting numeric SET arithmetic. Custom codecs opt in explicitly.
pub trait Numeric: WireType {}
impl Numeric for wire::Number {}
/// Native categories supporting top-level ADD.
pub trait Addable: WireType {}
impl Addable for wire::Number {}
impl Addable for wire::StringSet {}
impl Addable for wire::NumberSet {}
impl Addable for wire::BinarySet {}
/// Native set categories supporting top-level DELETE.
pub trait SetWire: Addable {}
impl SetWire for wire::StringSet {}
impl SetWire for wire::NumberSet {}
impl SetWire for wire::BinarySet {}
/// A list codec supplies the explicit logical empty value used by append_or_empty.
pub trait Append<T>: Converter<T, Wire = wire::List> {
    fn empty() -> T;
}
impl<T: Value> Append<List<T>> for Native<List<T>> {
    fn empty() -> List<T> {
        List(vec![])
    }
}

/// Inspectable SET grammar. Constants use Operand's redacted Debug.
#[derive(Debug, PartialEq)]
pub enum SetNode {
    Operand(Operand),
    IfNotExists {
        path: FieldDescriptor,
        default: Operand,
    },
    Plus(Box<Self>, Operand),
    Minus(Box<Self>, Operand),
    ListAppend(Box<Self>, Operand),
}
/// Read-only action tree; constructing nodes cannot bypass the typed builders.
#[derive(Debug, PartialEq)]
pub enum ActionNode {
    Set {
        target: FieldDescriptor,
        value: SetNode,
    },
    Remove {
        target: FieldDescriptor,
    },
    Add {
        target: FieldDescriptor,
        value: Operand,
    },
    Delete {
        target: FieldDescriptor,
        value: Operand,
    },
}

/// A validated, record-owned sequence. Rendering and target overlap checks are later stages.
pub struct BuiltUpdate<R> {
    actions: Vec<ActionNode>,
    owner: PhantomData<fn(R) -> R>,
}
impl<R> BuiltUpdate<R> {
    pub fn actions(&self) -> &[ActionNode] {
        &self.actions
    }
    pub fn into_actions(self) -> Vec<ActionNode> {
        self.actions
    }
}
impl<R> fmt::Debug for BuiltUpdate<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.actions.fmt(f)
    }
}

/// Owned actions; codecs run only at build. An empty update is rejected.
///
/// Required fields and protected targets have no removal/mutation methods:
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn required<R: Record>(a: Attr<R, u64>) { a.remove(); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, KeyOnly, Record, Required};
/// fn key<R: Record>(a: Attr<R, u64, Required, KeyOnly>) { a.set(1); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record, Required, VersionOnly};
/// fn version<R: Record>(a: Attr<R, u64, Required, VersionOnly>) { a.increment(1); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, IndexSourceOnly, Optional, Record};
/// fn source<R: Record>(a: Attr<R, u64, Optional, IndexSourceOnly>) { a.remove(); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, DerivedOnly, Record, Required};
/// fn derived<R: Record>(a: Attr<R, u64, Required, DerivedOnly>) { a.add(1); }
/// ```
/// Owners, logical types, and codecs must agree, including SET sources:
/// ```compile_fail
/// use tabulary::{Attr, Record, Update};
/// fn owner<R: Record, S: Record>(a: Attr<S, u64>) { Update::<R>::new().and(a.set(1)); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn source<R: Record, S: Record>(a: Attr<R, u64>, b: Attr<S, u64>) { a.set_expr(b.operand()); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Converter, Mutable, Record, Required, wire};
/// fn codec<R: Record, C: Converter<u64, Wire = wire::Number> + 'static>(
///     a: Attr<R, u64>, b: Attr<R, u64, Required, Mutable, C>,
/// ) { a.set_expr(b.if_not_exists(0)); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Optional, Record};
/// fn absent<R: Record>(a: Attr<R, u64, Optional>) { a.set(None); }
/// ```
/// Capabilities follow encoding, never the logical type alone:
/// ```compile_fail
/// use tabulary::{Attr, Mutable, Record, Required, convert::u128_string::Codec};
/// fn numeric<R: Record>(a: Attr<R, u128, Required, Mutable, Codec>) { a.increment(1); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn add<R: Record>(a: Attr<R, String>) { a.add("gift".into()); }
/// ```
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn delete<R: Record>(a: Attr<R, u64>) { a.delete_members(1); }
/// ```
/// DynamoDB allows one arithmetic operation, not recursive arithmetic:
/// ```compile_fail
/// use tabulary::{Attr, Record};
/// fn chained<R: Record>(a: Attr<R, u64>) { a.operand().plus(1).plus(2); }
/// ```
pub struct Update<R> {
    actions: Vec<Action<R>>,
}
impl<R> Default for Update<R> {
    fn default() -> Self {
        Self::new()
    }
}
impl<R> Update<R> {
    pub fn new() -> Self {
        Self { actions: vec![] }
    }
    pub fn and(mut self, action: Action<R>) -> Self {
        self.actions.push(action);
        self
    }
    pub fn build(self) -> Result<BuiltUpdate<R>, BuildError> {
        if self.actions.is_empty() {
            return Err(BuildError::EmptyOperation);
        }
        Ok(BuiltUpdate {
            actions: self
                .actions
                .into_iter()
                .map(|action| (action.build)())
                .collect::<Result<_, _>>()?,
            owner: PhantomData,
        })
    }
}
/// A single deferred action, constructed only through typed handles.
pub struct Action<R> {
    build: Box<dyn FnOnce() -> Result<ActionNode, BuildError>>,
    owner: PhantomData<fn(R) -> R>,
}
impl<R> Action<R> {
    fn new(build: impl FnOnce() -> Result<ActionNode, BuildError> + 'static) -> Self {
        Self {
            build: Box::new(build),
            owner: PhantomData,
        }
    }
}
macro_rules! unbuilt_debug {
    ($name:ident $(, $t:ident)*) => {
        impl<R, $($t,)*> fmt::Debug for $name<R, $($t,)*> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($name), "([unbuilt])")) }
        }
    };
}
unbuilt_debug!(Action);
unbuilt_debug!(Update);

type Invariant<T> = PhantomData<fn(T) -> T>;
/// A path or if_not_exists operand, preserving its record, logical type, and codec.
pub struct SetOperand<R, T, C = Native<T>> {
    field: Result<FieldDescriptor, BuildError>,
    build: Box<dyn FnOnce(FieldDescriptor) -> Result<SetNode, BuildError>>,
    types: Invariant<(R, T, C)>,
}
/// A complete SET right-hand side; arithmetic cannot be recursively chained.
pub struct SetExpression<R, T, C = Native<T>>(SetOperand<R, T, C>);
unbuilt_debug!(SetOperand, T, C);
unbuilt_debug!(SetExpression, T, C);
impl<R, T, C> From<SetOperand<R, T, C>> for SetExpression<R, T, C> {
    fn from(operand: SetOperand<R, T, C>) -> Self {
        Self(operand)
    }
}
impl<R, T, C> SetOperand<R, T, C> {
    fn new(
        field: Result<FieldDescriptor, BuildError>,
        build: impl FnOnce(FieldDescriptor) -> Result<SetNode, BuildError> + 'static,
    ) -> Self {
        Self {
            field,
            build: Box::new(build),
            types: PhantomData,
        }
    }
}
fn constant<R: Record, T, C: Converter<T>>(
    field: FieldDescriptor,
    value: &T,
) -> Result<Operand, BuildError> {
    let mut cx = EncodeContext::new(R::SCHEMA.name).at(PathSegment::Attribute(field.name.into()));
    Ok(Operand::Value(C::encode(value, &mut cx)?))
}
impl<R: Record, T: 'static, C: Converter<T> + 'static> SetOperand<R, T, C> {
    fn combine(
        self,
        value: T,
        node: fn(Box<SetNode>, Operand) -> SetNode,
    ) -> SetExpression<R, T, C> {
        SetExpression(Self::new(self.field, move |field| {
            Ok(node(
                Box::new((self.build)(field)?),
                constant::<R, T, C>(field, &value)?,
            ))
        }))
    }
    pub fn plus(self, delta: T) -> SetExpression<R, T, C>
    where
        C::Wire: Numeric,
    {
        self.combine(delta, SetNode::Plus)
    }
    pub fn minus(self, amount: T) -> SetExpression<R, T, C>
    where
        C::Wire: Numeric,
    {
        self.combine(amount, SetNode::Minus)
    }
}
impl<R: Record, T: 'static, P: PresenceMarker, A: AccessMarker, C: Converter<T> + 'static>
    Attr<R, T, P, A, C>
{
    /// Read a path on the SET right-hand side, including a protected attribute.
    pub fn operand(self) -> SetOperand<R, T, C> {
        SetOperand::new(self.validate(), |field| {
            Ok(SetNode::Operand(Operand::Attribute(field)))
        })
    }
    pub fn if_not_exists(self, value: T) -> SetOperand<R, T, C> {
        SetOperand::new(self.validate(), move |field| {
            Ok(SetNode::IfNotExists {
                path: field,
                default: constant::<R, T, C>(field, &value)?,
            })
        })
    }
}
impl<R: Record, T: 'static, P: PresenceMarker, C: Converter<T> + 'static>
    Attr<R, T, P, Mutable, C>
{
    pub fn set(self, value: T) -> Action<R> {
        self.set_expr(SetOperand::new(self.validate(), move |field| {
            Ok(SetNode::Operand(constant::<R, T, C>(field, &value)?))
        }))
    }
    pub fn set_expr(self, expression: impl Into<SetExpression<R, T, C>>) -> Action<R> {
        let target = self.validate_mutation();
        let SetOperand { field, build, .. } = expression.into().0;
        Action::new(move || {
            Ok(ActionNode::Set {
                target: target?,
                value: build(field?)?,
            })
        })
    }
    pub fn increment(self, delta: T) -> Action<R>
    where
        C::Wire: Numeric,
    {
        self.set_expr(self.operand().plus(delta))
    }
    pub fn decrement(self, amount: T) -> Action<R>
    where
        C::Wire: Numeric,
    {
        self.set_expr(self.operand().minus(amount))
    }
    pub fn list_append(self, value: T) -> Action<R>
    where
        C: Append<T>,
    {
        self.set_expr(self.operand().combine(value, SetNode::ListAppend))
    }
    pub fn append_or_empty(self, value: T) -> Action<R>
    where
        C: Append<T>,
    {
        self.set_expr(
            self.if_not_exists(C::empty())
                .combine(value, SetNode::ListAppend),
        )
    }
}
impl<R: Record, T: 'static, C: Converter<T> + 'static> Attr<R, T, Optional, Mutable, C> {
    pub fn remove(self) -> Action<R> {
        let target = self.validate_mutation();
        Action::new(move || Ok(ActionNode::Remove { target: target? }))
    }
}
impl<R: Record, T: Value + 'static, P: PresenceMarker> Attr<R, T, P, Mutable, Native<T>> {
    fn native_action(
        self,
        value: T,
        node: fn(FieldDescriptor, Operand) -> ActionNode,
    ) -> Action<R> {
        let target = self.validate_mutation();
        Action::new(move || {
            let target = target?;
            Ok(node(target, constant::<R, T, Native<T>>(target, &value)?))
        })
    }
    #[allow(clippy::should_implement_trait)] // DynamoDB ADD, not Rust arithmetic.
    pub fn add(self, value: T) -> Action<R>
    where
        T::Wire: Addable,
    {
        self.native_action(value, |target, value| ActionNode::Add { target, value })
    }
    pub fn delete_members(self, value: T) -> Action<R>
    where
        T::Wire: SetWire,
    {
        self.native_action(value, |target, value| ActionNode::Delete { target, value })
    }
}
