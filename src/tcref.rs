use std::str::FromStr;

use destream::{de, en, IntoStream};
use pathlink::PathBuf;

use crate::{Id, IdRef, Scalar};
use tc_value::Value;

/// A reference to a scalar value.
///
/// v2 currently supports op references (`TCRef::Op`), scope IDs (`TCRef::Id`), and basic flow
/// control (`TCRef::While`). Additional control-flow references (`If`, `Case`, etc.) will follow
/// once the kernel has a complete ref scheduler.
///
/// ## v1-compatible JSON semantics
///
/// Encoded as the underlying [`crate::OpRef`] map (no wrapper).
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum TCRef {
    Op(crate::op::OpRef),
    Id(IdRef),
    If(Box<IfRef>),
    Cond(Box<CondOp>),
    While(Box<While>),
    ForEach(Box<ForEach>),
}

/// A conditional reference (`if cond then then else or_else`).
#[derive(Clone, Debug, PartialEq)]
pub struct IfRef {
    pub cond: TCRef,
    pub then: Scalar,
    pub or_else: Scalar,
}

impl IfRef {
    pub fn new(cond: TCRef, then: Scalar, or_else: Scalar) -> Self {
        Self { cond, then, or_else }
    }
}

/// A lazy conditional reference: evaluate `cond` and execute only the selected branch OpDef.
#[derive(Clone, Debug, PartialEq)]
pub struct CondOp {
    pub cond: TCRef,
    pub then: crate::op::OpDef,
    pub or_else: crate::op::OpDef,
}

impl CondOp {
    pub fn new(cond: TCRef, then: crate::op::OpDef, or_else: crate::op::OpDef) -> Self {
        Self { cond, then, or_else }
    }
}

struct CondOpArgs {
    cond: Scalar,
    then: crate::op::OpDef,
    or_else: crate::op::OpDef,
}

impl de::FromStream for CondOpArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct CondOpArgsVisitor;

        impl de::Visitor for CondOpArgsVisitor {
            type Value = CondOpArgs;

            fn expecting() -> &'static str {
                "a CondOp args tuple"
            }

            async fn visit_seq<A: de::SeqAccess>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let cond = seq
                    .next_element::<Scalar>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("invalid CondOp params (missing condition)"))?;
                let then = seq
                    .next_element::<crate::op::OpDef>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("invalid CondOp params (missing then op)"))?;
                let or_else = seq
                    .next_element::<crate::op::OpDef>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("invalid CondOp params (missing else op)"))?;

                if seq.next_element::<de::IgnoredAny>(()).await?.is_some() {
                    return Err(de::Error::custom(
                        "invalid CondOp params (expected 3 elements)",
                    ));
                }

                Ok(CondOpArgs {
                    cond,
                    then,
                    or_else,
                })
            }
        }

        decoder.decode_seq(CondOpArgsVisitor).await
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
        match self {
            TCRef::Op(op) => op.into_stream(encoder),
            TCRef::Id(id_ref) => encode_id_ref(id_ref, encoder),
            TCRef::If(if_ref) => encode_if_ref(*if_ref, encoder),
            TCRef::Cond(cond_op) => encode_cond_op(*cond_op, encoder),
            TCRef::While(while_ref) => encode_while_ref(*while_ref, encoder),
            TCRef::ForEach(for_each) => encode_for_each_ref(*for_each, encoder),
        }
    }
}

impl<'en> en::ToStream<'en> for TCRef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
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
    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_IF)) {
        let items = map.next_value::<Vec<Scalar>>(()).await?;
        let mut iter = items.into_iter();
        let (cond, then, or_else) = match (iter.next(), iter.next(), iter.next(), iter.next()) {
            (Some(cond), Some(then), Some(or_else), None) => (cond, then, or_else),
            _ => {
                return Err(de::Error::custom(
                    "invalid If ref params (expected 3 elements)",
                ))
            }
        };

        let cond = match cond {
            Scalar::Ref(r) => *r,
            other => {
                return Err(de::Error::custom(format!(
                    "invalid If ref condition (expected ref, got {other:?})"
                )))
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::If(Box::new(IfRef::new(cond, then, or_else))));
    }

    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_COND)) {
        let args = map.next_value::<CondOpArgs>(()).await?;

        let cond = match args.cond {
            Scalar::Ref(r) => *r,
            other => {
                return Err(de::Error::custom(format!(
                    "invalid CondOp condition (expected ref, got {other:?})"
                )))
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::Cond(Box::new(CondOp::new(
            cond, args.then, args.or_else,
        ))));
    }

    if key_path.as_ref() == Some(&PathBuf::from(crate::TCREF_WHILE)) {
        let items = map.next_value::<Vec<Scalar>>(()).await?;
        let mut iter = items.into_iter();
        let (cond, closure, state) = match (iter.next(), iter.next(), iter.next(), iter.next()) {
            (Some(cond), Some(closure), Some(state), None) => (cond, closure, state),
            _ => {
                return Err(de::Error::custom(
                    "invalid While ref params (expected 3 elements)",
                ))
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
                ))
            }
        };

        let item_name = match item_name {
            Scalar::Value(Value::String(raw)) => raw
                .parse::<Id>()
                .map_err(|err| de::Error::custom(err.to_string()))?,
            other => {
                return Err(de::Error::custom(format!(
                    "invalid ForEach item_name (expected string, got {other:?})"
                )))
            }
        };

        while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
            let _ = map.next_value::<de::IgnoredAny>(()).await?;
        }

        return Ok(TCRef::ForEach(Box::new(ForEach::new(
            items, op, item_name,
        ))));
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

        let subject =
            crate::scalar::subject_from_str(&key).map_err(|err| de::Error::custom(err.to_string()))?;
        let op = crate::op::opref_from_subject_args(subject, args)?;
        return Ok(TCRef::Op(op));
    }

    let op = crate::op::decode_opref_map_entry(key, map).await?;
    Ok(TCRef::Op(op))
}

struct ScalarSeq(Vec<Scalar>);

impl ScalarSeq {
    fn new(items: Vec<Scalar>) -> Self {
        Self(items)
    }
}

impl<'en> en::IntoStream<'en> for ScalarSeq {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeSeq;

        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for item in self.0 {
            seq.encode_element(item)?;
        }
        seq.end()
    }
}

struct CondOpSeq {
    cond: TCRef,
    then: crate::op::OpDef,
    or_else: crate::op::OpDef,
}

impl CondOpSeq {
    fn new(cond: TCRef, then: crate::op::OpDef, or_else: crate::op::OpDef) -> Self {
        Self { cond, then, or_else }
    }
}

impl<'en> en::IntoStream<'en> for CondOpSeq {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeSeq;

        let mut seq = encoder.encode_seq(Some(3))?;
        seq.encode_element(Scalar::from(self.cond))?;
        seq.encode_element(self.then)?;
        seq.encode_element(self.or_else)?;
        seq.end()
    }
}

fn encode_id_ref<'en, E: en::Encoder<'en>>(id_ref: IdRef, encoder: E) -> Result<E::Ok, E::Error> {
    use destream::en::EncodeMap;

    let mut map = encoder.encode_map(Some(1))?;
    map.encode_key(id_ref.to_string())?;
    map.encode_value(ScalarSeq::new(Vec::new()))?;
    map.end()
}

fn encode_if_ref<'en, E: en::Encoder<'en>>(if_ref: IfRef, encoder: E) -> Result<E::Ok, E::Error> {
    use destream::en::EncodeMap;

    let mut map = encoder.encode_map(Some(1))?;
    map.encode_key(PathBuf::from(crate::TCREF_IF).to_string())?;
    map.encode_value(ScalarSeq::new(vec![
        Scalar::from(if_ref.cond),
        if_ref.then,
        if_ref.or_else,
    ]))?;
    map.end()
}

fn encode_cond_op<'en, E: en::Encoder<'en>>(cond_op: CondOp, encoder: E) -> Result<E::Ok, E::Error> {
    use destream::en::EncodeMap;

    let mut map = encoder.encode_map(Some(1))?;
    map.encode_key(PathBuf::from(crate::TCREF_COND).to_string())?;
    map.encode_value(CondOpSeq::new(cond_op.cond, cond_op.then, cond_op.or_else))?;
    map.end()
}

fn encode_while_ref<'en, E: en::Encoder<'en>>(
    while_ref: While,
    encoder: E,
) -> Result<E::Ok, E::Error> {
    use destream::en::EncodeMap;

    let mut map = encoder.encode_map(Some(1))?;
    map.encode_key(PathBuf::from(crate::TCREF_WHILE).to_string())?;
    map.encode_value(ScalarSeq::new(vec![
        while_ref.cond,
        while_ref.closure,
        while_ref.state,
    ]))?;
    map.end()
}

fn encode_for_each_ref<'en, E: en::Encoder<'en>>(
    for_each: ForEach,
    encoder: E,
) -> Result<E::Ok, E::Error> {
    use destream::en::EncodeMap;

    let mut map = encoder.encode_map(Some(1))?;
    map.encode_key(PathBuf::from(crate::TCREF_FOR_EACH).to_string())?;
    map.encode_value(ScalarSeq::new(vec![
        for_each.items,
        for_each.op,
        Scalar::Value(Value::String(for_each.item_name.to_string())),
    ]))?;
    map.end()
}
