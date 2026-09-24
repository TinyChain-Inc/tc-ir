use std::collections::BTreeSet;
use std::str::FromStr;

use async_hash::{Digest, Hash, Output};
use destream::{de, en};
use pathlink::PathBuf;

use crate::{Id, IdRef, Scalar};
use tc_value::Value;

/// A reference to a scalar value.
///
/// v2 supports op references (`TCRef::Op`), scope IDs (`TCRef::Id`), and flow control.
///
/// ## v1-compatible JSON semantics
///
/// Encoded as the underlying [`crate::OpRef`] map (no wrapper).
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum TCRef {
    Op(crate::op::OpRef),
    Id(IdRef),
    Cond(Box<Cond>),
    After(Box<After>),
    While(Box<While>),
    ForEach(Box<ForEach>),
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

impl TCRef {
    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        crate::scalar::collect_ref_requires(self, required)
    }

    pub(crate) fn visit_referenced_methods(
        &self,
        visitor: &mut impl FnMut(&pathlink::Link, crate::Method),
    ) {
        match self {
            Self::Op(op) => op.visit_referenced_methods(visitor),
            Self::Id(_) => {}
            Self::Cond(cond) => {
                cond.cond.visit_referenced_methods(visitor);
                cond.then.visit_referenced_methods(visitor);
                cond.or_else.visit_referenced_methods(visitor);
            }
            Self::After(after) => {
                after.when.visit_referenced_methods(visitor);
                after.then.visit_referenced_methods(visitor);
            }
            Self::While(while_ref) => {
                while_ref.cond.visit_referenced_methods(visitor);
                while_ref.closure.visit_referenced_methods(visitor);
                while_ref.state.visit_referenced_methods(visitor);
            }
            Self::ForEach(for_each) => {
                for_each.items.visit_referenced_methods(visitor);
                for_each.op.visit_referenced_methods(visitor);
            }
        }
    }
}

impl<D: Digest> Hash<D> for &TCRef {
    fn hash(self) -> Output<D> {
        match self {
            TCRef::Op(op) => Hash::<D>::hash(op),
            TCRef::Id(id) => Hash::<D>::hash(id),
            TCRef::Cond(cond) => Hash::<D>::hash((&cond.cond, &cond.then, &cond.or_else)),
            TCRef::After(after) => Hash::<D>::hash((&after.when, &after.then)),
            TCRef::While(while_ref) => {
                Hash::<D>::hash((&while_ref.cond, &while_ref.closure, &while_ref.state))
            }
            TCRef::ForEach(for_each) => {
                Hash::<D>::hash((&for_each.items, &for_each.op, &for_each.item_name))
            }
        }
    }
}

impl de::FromStream for TCRef {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct RefVisitor;

        impl de::Visitor for RefVisitor {
            type Value = TCRef;

            fn expecting() -> &'static str {
                "a Ref, like {\"$id\": []} or {\"/path/to/op\": [\"key\"]}"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let key = map
                    .next_key::<String>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("expected ref map key"))?;

                decode_tcref_map_entry(key, &mut map).await
            }
        }

        decoder.decode_map(RefVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TCRef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeMap;

        if let Self::Op(op) = self {
            return op.into_stream(encoder);
        }
        let mut map = encoder.encode_map(Some(1))?;
        match self {
            Self::Op(_) => unreachable!(),
            Self::Id(id) => map.encode_entry(id.to_string(), Vec::<()>::new())?,
            Self::Cond(cond) => map.encode_entry(
                PathBuf::from(crate::TCREF_COND).to_string(),
                (cond.cond, cond.then, cond.or_else),
            )?,
            Self::After(after) => map.encode_entry(
                PathBuf::from(crate::TCREF_AFTER).to_string(),
                (after.when, after.then),
            )?,
            Self::While(while_ref) => map.encode_entry(
                PathBuf::from(crate::TCREF_WHILE).to_string(),
                (while_ref.cond, while_ref.closure, while_ref.state),
            )?,
            Self::ForEach(for_each) => map.encode_entry(
                PathBuf::from(crate::TCREF_FOR_EACH).to_string(),
                (for_each.items, for_each.op, for_each.item_name.to_string()),
            )?,
        }
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TCRef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeMap;

        if let Self::Op(op) = self {
            return op.to_stream(encoder);
        }
        let mut map = encoder.encode_map(Some(1))?;
        match self {
            Self::Op(_) => unreachable!(),
            Self::Id(id) => map.encode_entry(id.to_string(), Vec::<()>::new())?,
            Self::Cond(cond) => map.encode_entry(
                PathBuf::from(crate::TCREF_COND).to_string(),
                (&cond.cond, &cond.then, &cond.or_else),
            )?,
            Self::After(after) => map.encode_entry(
                PathBuf::from(crate::TCREF_AFTER).to_string(),
                (&after.when, &after.then),
            )?,
            Self::While(while_ref) => map.encode_entry(
                PathBuf::from(crate::TCREF_WHILE).to_string(),
                (&while_ref.cond, &while_ref.closure, &while_ref.state),
            )?,
            Self::ForEach(for_each) => map.encode_entry(
                PathBuf::from(crate::TCREF_FOR_EACH).to_string(),
                (
                    &for_each.items,
                    &for_each.op,
                    for_each.item_name.to_string(),
                ),
            )?,
        }
        map.end()
    }
}

pub(crate) async fn decode_tcref_map_entry<A: de::MapAccess>(
    key: String,
    map: &mut A,
) -> Result<TCRef, A::Error> {
    let key_path = if key.starts_with('/') {
        PathBuf::from_str(&key).ok()
    } else {
        None
    };
    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_COND)) {
        let items = map.next_value::<Vec<Scalar>>(()).await?;
        let mut iter = items.into_iter();
        let (cond, then, or_else) = match (iter.next(), iter.next(), iter.next(), iter.next()) {
            (Some(cond), Some(then), Some(or_else), None) => (cond, then, or_else),
            _ => {
                return Err(de::Error::custom(
                    "invalid Cond params (expected 3 elements)",
                ));
            }
        };

        let cond = match cond {
            Scalar::Ref(r) => *r,
            other => {
                return Err(de::Error::custom(format!(
                    "invalid Cond condition (expected ref, got {other:?})"
                )));
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::Cond(Box::new(Cond::new(cond, then, or_else))));
    }

    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_AFTER)) {
        let items = map.next_value::<Vec<Scalar>>(()).await?;
        let mut iter = items.into_iter();
        let (when, then) = match (iter.next(), iter.next(), iter.next()) {
            (Some(when), Some(then), None) => (when, then),
            _ => {
                return Err(de::Error::custom(
                    "invalid After params (expected 2 elements)",
                ));
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::After(Box::new(After::new(when, then))));
    }

    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_WHILE)) {
        let items = map.next_value::<Vec<Scalar>>(()).await?;
        let mut iter = items.into_iter();
        let (cond, closure, state) = match (iter.next(), iter.next(), iter.next(), iter.next()) {
            (Some(cond), Some(closure), Some(state), None) => (cond, closure, state),
            _ => {
                return Err(de::Error::custom(
                    "invalid While ref params (expected 3 elements)",
                ));
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::While(Box::new(While::new(cond, closure, state))));
    }

    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_FOR_EACH)) {
        let items = map.next_value::<Vec<Scalar>>(()).await?;
        let mut iter = items.into_iter();
        let (items, op, item_name) = match (iter.next(), iter.next(), iter.next(), iter.next()) {
            (Some(items), Some(op), Some(item_name), None) => (items, op, item_name),
            _ => {
                return Err(de::Error::custom(
                    "invalid ForEach ref params (expected 3 elements)",
                ));
            }
        };

        let item_name = match item_name {
            Scalar::Value(Value::String(raw)) => raw
                .parse::<Id>()
                .map_err(|err| de::Error::custom(err.to_string()))?,
            other => {
                return Err(de::Error::custom(format!(
                    "invalid ForEach item_name (expected string, got {other:?})"
                )));
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::ForEach(Box::new(ForEach::new(items, op, item_name))));
    }

    if key.starts_with('$') {
        let args = map.next_value::<crate::op::OpArgs>(()).await?;
        if let crate::op::OpArgs::Seq(items) = &args {
            if items.is_empty() {
                let id_ref =
                    IdRef::from_str(&key).map_err(|err| de::Error::custom(err.to_string()))?;
                return Ok(TCRef::Id(id_ref));
            }
        }

        let subject = crate::scalar::subject_from_str(&key)
            .map_err(|err| de::Error::custom(err.to_string()))?;
        let op = crate::op::opref_from_subject_args(subject, args)?;
        return Ok(TCRef::Op(op));
    }

    let op = crate::op::decode_opref_map_entry(key, map).await?;
    Ok(TCRef::Op(op))
}
