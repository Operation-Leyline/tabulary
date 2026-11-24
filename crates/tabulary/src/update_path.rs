//! Typed physical paths; no segment is interpreted as expression syntax.
use crate::{
    AccessMarker, Attr, AttributePath, BuildError, Condition, Converter, Document, EncodeContext,
    FieldAccess, FieldDescriptor, List, Map, Mutable, Native, Optional, PathSegment,
    PresenceMarker, Record, Required, Value,
    condition::{
        Contains, Operand, Operator, Ordered, Prefix, Size, SizedWire, Subject, comparisons,
        ordered_comparisons, predicate,
    },
    update::{
        Action, ActionNode, Addable, Append, Numeric, SetExpression, SetNode, SetOperand, constant,
    },
};
use std::{fmt, marker::PhantomData};

/// A nonempty dynamic data key. Dots and brackets remain literal characters.
pub struct MapKey(String);
impl MapKey {
    pub fn new(key: impl Into<String>) -> Result<Self, BuildError> {
        let key = key.into();
        if key.is_empty() {
            return Err(BuildError::OperationConstraint {
                code: "empty_map_key",
            });
        }
        Ok(Self(key))
    }
}
/// Presence marker for an explicitly addressed list element.
pub struct Element;

#[derive(Clone)]
pub(crate) struct UpdatePath {
    pub(crate) path: AttributePath,
    pub(crate) document: &'static str,
    field: FieldDescriptor,
    writable: bool,
}
impl UpdatePath {
    pub(crate) fn root<R: Record>(field: FieldDescriptor) -> Self {
        Self {
            path: vec![PathSegment::Attribute(field.name.into())].into(),
            document: R::SCHEMA.name,
            writable: field.access == FieldAccess::Mutable
                && field.index != R::SCHEMA.primary.partition
                && Some(field.index) != R::SCHEMA.primary.sort,
            field,
        }
    }
    /// Encode constants at the complete physical path, inside the leaf's document.
    pub(crate) fn context<R: Record>(&self) -> EncodeContext {
        let mut cx = EncodeContext::new(R::SCHEMA.name).in_document(self.document);
        for segment in self.path.segments() {
            cx = cx.at(segment.clone());
        }
        cx
    }
    pub(crate) fn operand(&self) -> SetNode {
        if self.path.segments().len() == 1 {
            SetNode::Operand(crate::condition::Operand::Attribute(self.field))
        } else {
            SetNode::Path(self.path.clone())
        }
    }
    pub(crate) fn if_not_exists(self, default: crate::condition::Operand) -> SetNode {
        if self.path.segments().len() == 1 {
            SetNode::IfNotExists {
                path: self.field,
                default,
            }
        } else {
            SetNode::IfPathNotExists {
                path: self.path,
                default,
            }
        }
    }
    fn push(mut self, segment: PathSegment) -> Self {
        let mut segments = self.path.segments().to_vec();
        segments.push(segment);
        self.path = segments.into();
        self
    }
    fn mutation(self) -> Result<Self, BuildError> {
        if !self.writable {
            return Err(BuildError::OperationConstraint {
                code: "protected_update_path",
            });
        }
        Ok(self)
    }
}

type Invariant<T> = PhantomData<fn(T) -> T>;

/// Root-owned nested path handle for conditions and updates. Every attribute
/// segment is aliased separately when rendered (`#n0.#n1`), so reserved words and
/// literal dots in wire names are safe. Required fields have no remove method,
/// ADD is available on document fields and map entries but not list elements,
/// and DELETE remains top-level only.
/// ```compile_fail
/// use tabulary::{List, Attr, Record};
/// fn nested<R: Record>(a: Attr<R, List<u64>>) { a.at(0).add(1); }
/// ```
/// ```compile_fail
/// use tabulary::{Document, Value, Attr, Record};
/// fn required<R: Record, D: Document + Value + 'static>(a: Attr<R, D>, b: Attr<D, u64>) {
///     a.field(b).remove();
/// }
/// ```
/// ```compile_fail
/// use tabulary::{Document, Value, Attr, Record};
/// fn text<R: Record, D: Document + Value + 'static>(a: Attr<R, D>, b: Attr<D, String>) {
///     a.field(b).add("gift".to_owned());
/// }
/// ```
/// ```compile_fail
/// use tabulary::{Document, Value, Attr, Record};
/// fn ordering<R: Record, D: Document + Value + 'static>(a: Attr<R, D>, b: Attr<D, bool>) {
///     a.field(b).lt(true);
/// }
/// ```
pub struct Path<R, T, P = Required, A = Mutable, C = Native<T>> {
    path: Result<UpdatePath, BuildError>,
    types: Invariant<(R, T, P, A, C)>,
}
impl<R, T, P, A, C> Clone for Path<R, T, P, A, C> {
    fn clone(&self) -> Self {
        Self::new(self.path.clone())
    }
}
impl<R, T, P, A, C> fmt::Debug for Path<R, T, P, A, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Path([typed])")
    }
}
impl<R, T, P, A, C> Path<R, T, P, A, C> {
    fn new(path: Result<UpdatePath, BuildError>) -> Self {
        Self {
            path,
            types: PhantomData,
        }
    }
}
impl<R: Record, T: 'static, P, A, C: Converter<T> + 'static> Path<R, T, P, A, C> {
    pub fn operand(self) -> SetOperand<R, T, C> {
        SetOperand::new(self.path, |path| Ok(path.operand()))
    }
    pub fn if_not_exists(self, value: T) -> SetOperand<R, T, C> {
        SetOperand::new(self.path, move |path| {
            let default = constant::<R, T, C>(&path, &value)?;
            Ok(path.if_not_exists(default))
        })
    }
}
impl<R: Record, T: 'static, P, A, C: Converter<T> + 'static> Path<R, T, P, A, C> {
    fn subject(self) -> Result<Subject, BuildError> {
        self.path.map(Subject::Path)
    }
    fn predicate(self, operator: Operator, values: Vec<T>) -> Condition<R> {
        predicate::<R, T, C>(self.subject(), false, operator, values)
    }
    comparisons!(T; eq => Eq, ne => Ne);
    /// `attribute_exists` on the full path; false when any ancestor map is absent.
    pub fn exists(self) -> Condition<R> {
        self.predicate(Operator::Exists, vec![])
    }
    /// `attribute_not_exists` on the full path; true when any ancestor map is absent.
    pub fn not_exists(self) -> Condition<R> {
        self.predicate(Operator::NotExists, vec![])
    }
    pub fn in_values(self, values: Vec<T>) -> Condition<R> {
        self.predicate(Operator::In, values)
    }
    /// Compare two paths of the same owner, logical type, and codec.
    pub fn eq_path<P2, A2>(self, other: Path<R, T, P2, A2, C>) -> Condition<R> {
        let (left, right) = (self.path, other.path);
        Condition::new(move || {
            Ok(crate::condition::ConditionNode::Predicate {
                operator: Operator::Eq,
                operands: vec![Operand::Path(left?.path), Operand::Path(right?.path)],
            })
        })
    }
    pub fn begins_with(self, value: T) -> Condition<R>
    where
        C::Wire: Prefix,
    {
        self.predicate(Operator::BeginsWith, vec![value])
    }
    pub fn size(self) -> Size<R>
    where
        C::Wire: SizedWire,
    {
        Size {
            field: self.subject(),
            guard: None,
            owner: PhantomData,
        }
    }
    pub fn contains(self, value: C::Element) -> Condition<R>
    where
        C: Contains<T>,
    {
        predicate::<R, C::Element, C::ElementCodec>(
            self.subject(),
            false,
            Operator::Contains,
            vec![value],
        )
    }
}
impl<R: Record, T: 'static, P, A, C: Converter<T> + 'static> Path<R, T, P, A, C>
where
    C::Wire: Ordered,
{
    ordered_comparisons!(T);
}
impl<R: Record, T: 'static, P, C: Converter<T> + 'static> Path<R, T, P, Mutable, C> {
    pub fn set(self, value: T) -> Action<R> {
        let operand = SetOperand::new(self.path.clone(), move |path| {
            Ok(SetNode::Operand(constant::<R, T, C>(&path, &value)?))
        });
        self.set_expr(operand)
    }
    pub fn set_expr(self, expression: impl Into<SetExpression<R, T, C>>) -> Action<R> {
        let expression = expression.into().0;
        Action::new(move || {
            let target = self.path?.mutation()?.path;
            let value = (expression.build)(expression.field?)?;
            Ok(ActionNode::NestedSet { target, value })
        })
    }
    /// `SET path = if_not_exists(path, value)`. The parent map must already exist.
    pub fn set_if_not_exists(self, value: T) -> Action<R> {
        let rhs = self.clone().if_not_exists(value);
        self.set_expr(rhs)
    }
    pub fn increment(self, value: T) -> Action<R>
    where
        C::Wire: Numeric,
    {
        let rhs = self.clone().operand().plus(value);
        self.set_expr(rhs)
    }
    pub fn decrement(self, value: T) -> Action<R>
    where
        C::Wire: Numeric,
    {
        let rhs = self.clone().operand().minus(value);
        self.set_expr(rhs)
    }
    pub fn list_append(self, value: T) -> Action<R>
    where
        C: Append<T>,
    {
        let rhs = self.clone().operand().combine(value, SetNode::ListAppend);
        self.set_expr(rhs)
    }
    pub fn append_or_empty(self, value: T) -> Action<R>
    where
        C: Append<T>,
    {
        let rhs = self
            .clone()
            .if_not_exists(C::empty())
            .combine(value, SetNode::ListAppend);
        self.set_expr(rhs)
    }
    fn remove_action(self) -> Action<R> {
        Action::new(move || {
            Ok(ActionNode::NestedRemove {
                target: self.path?.mutation()?.path,
            })
        })
    }
}
impl<R: Record, T: 'static, C: Converter<T> + 'static> Path<R, T, Optional, Mutable, C> {
    pub fn remove(self) -> Action<R> {
        self.remove_action()
    }
}
impl<R: Record, T: Value + 'static, P: PresenceMarker> Path<R, T, P, Mutable, Native<T>> {
    /// DynamoDB ADD on a nested document field or map entry. An absent leaf is
    /// initialized by service semantics; an absent parent map is rejected.
    #[allow(clippy::should_implement_trait)] // DynamoDB ADD, not Rust arithmetic.
    pub fn add(self, value: T) -> Action<R>
    where
        T::Wire: Addable,
    {
        Action::new(move || {
            let path = self.path?.mutation()?;
            let value = constant::<R, T, Native<T>>(&path, &value)?;
            Ok(ActionNode::NestedAdd {
                target: path.path,
                value,
            })
        })
    }
}
impl<R: Record, T: 'static, C: Converter<T> + 'static> Path<R, T, Element, Mutable, C> {
    pub fn remove_element(self) -> Action<R> {
        self.remove_action()
    }
}
impl<R, T: Document + Value, P, A> Path<R, T, P, A> {
    pub fn field<U, Q: PresenceMarker, B: AccessMarker, C: Converter<U>>(
        self,
        field: Attr<T, U, Q, B, C>,
    ) -> Path<R, U, Q, B, C> {
        Path::new(self.path.and_then(|path| {
            let field = field.descriptor();
            if T::FIELDS.get(field.index) != Some(&field) {
                return Err(BuildError::OperationConstraint {
                    code: "invalid_attribute_descriptor",
                });
            }
            let mut path = path.push(PathSegment::Attribute(field.name.into()));
            path.writable &= field.access == FieldAccess::Mutable;
            path.document = T::NAME;
            Ok(path)
        }))
    }
}
impl<R, T: Value, P, A> Path<R, List<T>, P, A> {
    pub fn at<I: TryInto<usize>>(self, index: I) -> Path<R, T, Element, A> {
        Path::new(self.path.and_then(|path| {
            Ok(path.push(PathSegment::Index(index.try_into().map_err(|_| {
                BuildError::OperationConstraint {
                    code: "invalid_list_index",
                }
            })?)))
        }))
    }
}
impl<R, T: Value, P, A> Path<R, Map<T>, P, A> {
    pub fn entry(self, key: MapKey) -> Path<R, T, Optional, A> {
        Path::new(
            self.path
                .map(|path| path.push(PathSegment::Attribute(key.0))),
        )
    }
}
impl<R: Record, T: Value, P: PresenceMarker, A: AccessMarker> Attr<R, T, P, A> {
    fn path(self) -> Path<R, T, P, A> {
        Path::new(self.validate().map(UpdatePath::root::<R>))
    }
}
impl<R: Record, T: Document + Value, P: PresenceMarker, A: AccessMarker> Attr<R, T, P, A> {
    pub fn field<U, Q: PresenceMarker, B: AccessMarker, C: Converter<U>>(
        self,
        field: Attr<T, U, Q, B, C>,
    ) -> Path<R, U, Q, B, C> {
        self.path().field(field)
    }
}
impl<R: Record, T: Value, P: PresenceMarker, A: AccessMarker> Attr<R, List<T>, P, A> {
    pub fn at<I: TryInto<usize>>(self, index: I) -> Path<R, T, Element, A> {
        self.path().at(index)
    }
}
impl<R: Record, T: Value, P: PresenceMarker, A: AccessMarker> Attr<R, Map<T>, P, A> {
    pub fn entry(self, key: MapKey) -> Path<R, T, Optional, A> {
        self.path().entry(key)
    }
}
