use std::collections::BTreeSet;

use async_hash::{Digest, Hash, Output};

use crate::{Id, IdRef, Scalar};

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
        crate::scalar::collect_ref_requires(self, required)
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
