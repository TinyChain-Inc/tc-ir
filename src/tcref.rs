use std::collections::BTreeSet;

use async_hash::{Digest, Hash, Output};

use crate::{Id, IdRef, Scalar, Subject};
use tc_error::TCResult;

/// A reference to a scalar value, with ordinary recursive ownership.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq)]
pub enum TCRef {
    Op(crate::OpRef),
    Id(IdRef),
    Cond(Box<Cond>),
    After(Box<After>),
    While(Box<While>),
    ForEach(Box<ForEach>),
}

impl std::fmt::Debug for TCRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Op(crate::OpRef::Post((subject, params))) => {
                write!(f, "Op(Post(({subject:?}, {:?})))", **params)
            }
            Self::Op(value) => write!(f, "Op({value:?})"),
            Self::Id(value) => write!(f, "Id({value:?})"),
            Self::Cond(value) => write!(f, "Cond({value:?})"),
            Self::After(value) => write!(f, "After({value:?})"),
            Self::While(value) => write!(f, "While({value:?})"),
            Self::ForEach(value) => write!(f, "ForEach({value:?})"),
        }
    }
}

/// A conditional reference with scalar branches.
#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub cond: TCRef,
    pub then: Scalar,
    pub or_else: Scalar,
}

impl Cond {
    pub fn new(cond: TCRef, then: Scalar, or_else: Scalar) -> Self {
        Self {
            cond,
            then,
            or_else,
        }
    }
}

/// Resolve `when` before resolving and returning `then`.
#[derive(Clone, Debug, PartialEq)]
pub struct After {
    pub when: Scalar,
    pub then: Scalar,
}

impl After {
    pub fn new(when: Scalar, then: Scalar) -> Self {
        Self { when, then }
    }
}

/// A `While` loop reference: repeatedly resolve `closure` while `cond` is `true`.
#[derive(Clone, Debug, PartialEq)]
pub struct While {
    pub cond: Scalar,
    pub closure: Scalar,
    pub state: Scalar,
}

impl While {
    pub fn new(cond: Scalar, closure: Scalar, state: Scalar) -> Self {
        Self {
            cond,
            closure,
            state,
        }
    }
}

/// A `ForEach` reference: apply `op` to each item in `items`.
#[derive(Clone, Debug, PartialEq)]
pub struct ForEach {
    pub items: Scalar,
    pub op: Scalar,
    pub item_name: Id,
}

impl ForEach {
    pub fn new(items: Scalar, op: Scalar, item_name: Id) -> Self {
        Self {
            items,
            op,
            item_name,
        }
    }
}

impl get_size::GetSize for TCRef {
    fn get_heap_size(&self) -> usize {
        match self {
            Self::Id(value) => value.get_heap_size(),
            Self::Op(value) => value.get_heap_size(),
            Self::Cond(value) => {
                std::mem::size_of::<Cond>()
                    + value.cond.get_heap_size()
                    + value.then.get_heap_size()
                    + value.or_else.get_heap_size()
            }
            Self::After(value) => {
                std::mem::size_of::<After>()
                    + value.when.get_heap_size()
                    + value.then.get_heap_size()
            }
            Self::While(value) => {
                std::mem::size_of::<While>()
                    + value.cond.get_heap_size()
                    + value.closure.get_heap_size()
                    + value.state.get_heap_size()
            }
            Self::ForEach(value) => {
                std::mem::size_of::<ForEach>()
                    + value.items.get_heap_size()
                    + value.op.get_heap_size()
                    + value.item_name.get_heap_size()
            }
        }
    }
}

impl TCRef {
    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        self.requires_in(&BTreeSet::new(), required)
    }

    pub(crate) fn requires_in(&self, bound: &BTreeSet<Id>, required: &mut BTreeSet<Id>) {
        match self {
            Self::Id(id) => {
                if id.as_str() != "self" && !bound.contains(id.id()) {
                    required.insert(id.id().clone());
                }
            }
            Self::Op(op) => op.requires_in(bound, required),
            Self::Cond(value) => {
                value.cond.requires_in(bound, required);
                value.then.requires_in(bound, required);
                value.or_else.requires_in(bound, required);
            }
            Self::After(value) => {
                value.when.requires_in(bound, required);
                value.then.requires_in(bound, required);
            }
            Self::While(value) => {
                let mut callback = bound.clone();
                callback.insert("state".parse().expect("reserved callback Id"));
                value.cond.requires_in(&callback, required);
                value.closure.requires_in(&callback, required);
                value.state.requires_in(bound, required);
            }
            Self::ForEach(value) => {
                let mut body = bound.clone();
                body.insert(value.item_name.clone());
                value.op.requires_in(&body, required);
                value.items.requires_in(bound, required);
            }
        }
    }

    pub(crate) fn validate_in(&self, visible: &BTreeSet<Id>) -> TCResult<()> {
        match self {
            Self::Id(_) => {}
            Self::Op(op) => op.validate_in(visible)?,
            Self::Cond(value) => {
                value.cond.validate_in(visible)?;
                value.then.validate_in(visible)?;
                value.or_else.validate_in(visible)?;
            }
            Self::After(value) => {
                value.when.validate_in(visible)?;
                value.then.validate_in(visible)?;
            }
            Self::While(value) => {
                let mut callback = visible.clone();
                crate::op::bind(
                    &mut callback,
                    &mut BTreeSet::new(),
                    &"state".parse().expect("reserved callback Id"),
                    "While callback parameter",
                )?;
                value.cond.validate_in(&callback)?;
                value.closure.validate_in(&callback)?;
                value.state.validate_in(visible)?;
            }
            Self::ForEach(value) => {
                let mut body = visible.clone();
                crate::op::bind(
                    &mut body,
                    &mut BTreeSet::new(),
                    &value.item_name,
                    "ForEach item",
                )?;
                value.op.validate_in(&body)?;
                value.items.validate_in(visible)?;
            }
        }
        Ok(())
    }

    pub(crate) fn map_subjects(&mut self, visitor: &mut impl FnMut(&mut Subject)) {
        match self {
            Self::Id(_) => {}
            Self::Op(op) => op.map_subjects(visitor),
            Self::Cond(value) => {
                value.or_else.map_subjects(visitor);
                value.then.map_subjects(visitor);
                value.cond.map_subjects(visitor);
            }
            Self::After(value) => {
                value.then.map_subjects(visitor);
                value.when.map_subjects(visitor);
            }
            Self::While(value) => {
                value.state.map_subjects(visitor);
                value.closure.map_subjects(visitor);
                value.cond.map_subjects(visitor);
            }
            Self::ForEach(value) => {
                value.op.map_subjects(visitor);
                value.items.map_subjects(visitor);
            }
        }
    }

    pub(crate) fn visit_referenced_methods(
        &self,
        visitor: &mut impl FnMut(&pathlink::Link, crate::Method),
    ) {
        match self {
            Self::Id(_) => {}
            Self::Op(op) => op.visit_referenced_methods(visitor),
            Self::Cond(value) => {
                value.cond.visit_referenced_methods(visitor);
                value.then.visit_referenced_methods(visitor);
                value.or_else.visit_referenced_methods(visitor);
            }
            Self::After(value) => {
                value.when.visit_referenced_methods(visitor);
                value.then.visit_referenced_methods(visitor);
            }
            Self::While(value) => {
                value.cond.visit_referenced_methods(visitor);
                value.closure.visit_referenced_methods(visitor);
                value.state.visit_referenced_methods(visitor);
            }
            Self::ForEach(value) => {
                value.items.visit_referenced_methods(visitor);
                value.op.visit_referenced_methods(visitor);
            }
        }
    }
}

impl<D: Digest> Hash<D> for &TCRef {
    fn hash(self) -> Output<D> {
        match self {
            TCRef::Id(value) => Hash::<D>::hash(value),
            TCRef::Op(value) => Hash::<D>::hash(value),
            TCRef::Cond(value) => Hash::<D>::hash((&value.cond, &value.then, &value.or_else)),
            TCRef::After(value) => Hash::<D>::hash((&value.when, &value.then)),
            TCRef::While(value) => Hash::<D>::hash((&value.cond, &value.closure, &value.state)),
            TCRef::ForEach(value) => Hash::<D>::hash((&value.items, &value.op, &value.item_name)),
        }
    }
}
