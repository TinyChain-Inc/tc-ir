use std::str::FromStr;

use destream::de::{self, FromStream};
use pathlink::{Link, PathBuf};

use crate::{
    op::OpDefType, After, Cond, ForEach, Id, Map, OpDef, OpRef, Scalar, Subject, TCRef, While,
};
use tc_value::Value;

fn descend<E: de::Error>(remaining: usize, levels: usize) -> Result<usize, E> {
    remaining.checked_sub(levels).ok_or_else(|| {
        E::custom(format!(
            "IR structural nesting exceeds {}",
            crate::MAX_SCALAR_DEPTH
        ))
    })
}

fn require_field<E: de::Error>(
    sequence: &de::Container,
    index: usize,
    count: usize,
) -> Result<(), E> {
    if sequence.slot() == de::Slot::End {
        Err(E::invalid_length(index, count))
    } else {
        Ok(())
    }
}

fn end_fields<E: de::Error>(sequence: &de::Container) -> Result<(), E> {
    if sequence.slot() == de::Slot::End {
        Ok(())
    } else {
        Err(E::custom("too many operation fields"))
    }
}

async fn field<T: FromStream<Context = ()>, D: de::Decoder>(
    decoder: &mut D,
    sequence: &mut de::Container,
    index: usize,
    count: usize,
) -> Result<T, D::Error> {
    require_field::<D::Error>(sequence, index, count)?;
    let value = T::from_stream((), decoder).await?;
    decoder.finish_child(sequence).await?;
    Ok(value)
}

// Box this helper at recursive callers: its delimiter future must not inflate
// every enclosing reference frame. Keep the child Scalar future boxed as well.
async fn scalar_field<D: de::Decoder>(
    decoder: &mut D,
    sequence: &mut de::Container,
    remaining: usize,
    index: usize,
    count: usize,
) -> Result<Scalar, D::Error> {
    require_field::<D::Error>(sequence, index, count)?;
    let value = Box::pin(decode_scalar(decoder, remaining)).await?;
    decoder.finish_child(sequence).await?;
    Ok(value)
}

async fn first_key<D: de::Decoder>(
    decoder: &mut D,
    map: &mut de::Container,
) -> Result<String, D::Error> {
    if map.slot() == de::Slot::End {
        return Err(de::Error::custom("expected nonempty IR type map"));
    }
    let key = String::from_stream((), decoder).await?;
    decoder.finish_child(map).await?;
    Ok(key)
}

async fn finish_map<D: de::Decoder>(
    decoder: &mut D,
    mut map: de::Container,
    drain: bool,
) -> Result<(), D::Error> {
    while map.slot() != de::Slot::End {
        if !drain {
            return Err(de::Error::custom("unexpected extra reference map entry"));
        }
        de::IgnoredAny::from_stream((), decoder).await?;
        decoder.finish_child(&mut map).await?;
    }

    Ok(())
}

async fn scalar_items<D: de::Decoder>(
    decoder: &mut D,
    mut sequence: de::Container,
    remaining: usize,
) -> Result<Vec<Scalar>, D::Error> {
    let mut values = Vec::new();
    while sequence.slot() != de::Slot::End {
        let value = Box::pin(decode_scalar(decoder, remaining)).await?;
        values.try_reserve(1).map_err(de::Error::custom)?;
        values.push(value);
        decoder.finish_child(&mut sequence).await?;
    }

    Ok(values)
}

async fn scalar_map<D: de::Decoder>(
    decoder: &mut D,
    mut map: de::Container,
    mut key: Option<String>,
    remaining: usize,
    unique: bool,
) -> Result<Map<Scalar>, D::Error> {
    let mut values = Map::new();
    while map.slot() != de::Slot::End {
        let name = match key.take() {
            Some(key) => key,
            None => first_key(decoder, &mut map).await?,
        };
        let value = Box::pin(decode_scalar(decoder, remaining)).await?;
        let id = name.parse::<Id>().map_err(de::Error::custom)?;
        if values.insert(id.clone(), value).is_some() && unique {
            return Err(de::Error::custom(format!("duplicate map key {id}")));
        }
        decoder.finish_child(&mut map).await?;
    }

    Ok(values)
}

// Subject syntax uses a sequence for GET/PUT and a map for POST. Empty
// sequences can instead represent a literal Link or an Id reference.
async fn args<D: de::Decoder>(
    decoder: &mut D,
    remaining: usize,
    levels: usize,
) -> Result<Scalar, D::Error> {
    match decoder.peek_kind().await? {
        de::Kind::Seq => {
            let sequence = decoder.open_container(de::Kind::Seq, None).await?;
            if sequence.slot() == de::Slot::End {
                return Ok(Scalar::Tuple(Vec::new()));
            }
            let remaining = descend::<D::Error>(remaining, levels)?;
            Ok(Scalar::Tuple(
                scalar_items(decoder, sequence, remaining).await?,
            ))
        }
        de::Kind::Map => {
            let remaining = descend::<D::Error>(remaining, levels)?;
            let map = decoder.open_container(de::Kind::Map, None).await?;
            Ok(Scalar::Map(
                scalar_map(decoder, map, None, remaining, false).await?,
            ))
        }
        _ => Err(de::Error::custom("invalid IR container shape")),
    }
}

fn call<E: de::Error>(key: &str, args: Scalar) -> Result<OpRef, E> {
    let subject = crate::scalar::subject_from_str(key).map_err(E::custom)?;
    match args {
        Scalar::Map(params) => Ok(OpRef::Post((subject, params))),
        Scalar::Tuple(items) => {
            let len = items.len();
            let mut items = items.into_iter();
            match len {
                1 => Ok(OpRef::Get((subject, items.next().unwrap()))),
                2 => Ok(OpRef::Put((
                    subject,
                    items.next().unwrap(),
                    items.next().unwrap(),
                ))),
                _ => Err(E::custom("invalid OpRef params (expected 1 or 2 elements)")),
            }
        }
        _ => Err(E::custom("invalid IR container shape")),
    }
}

fn reference_path(path: &[Id]) -> bool {
    // Compare borrowed segments, as for OpDef types; do not allocate tag paths.
    (path.len() == 4
        && path[..3] == crate::SCALAR_REF_PREFIX[..]
        && matches!(path[3].as_str(), "cond" | "after" | "while" | "for_each"))
        || (path.len() == 5
            && path[..4] == crate::OPREF_PREFIX[..]
            && matches!(path[4].as_str(), "get" | "put" | "post" | "delete"))
}

/// Decode a Scalar within the remaining structural depth of its owner.
pub async fn decode_scalar<D: de::Decoder>(
    decoder: &mut D,
    remaining: usize,
) -> Result<Scalar, D::Error> {
    let remaining = remaining.min(crate::MAX_SCALAR_DEPTH);
    match decoder.peek_kind().await? {
        // Keep the nested Value codec out of each recursive Scalar frame.
        de::Kind::Leaf => Ok(Box::pin(tc_value::decode_value(decoder, remaining))
            .await?
            .into()),
        de::Kind::Seq => {
            let remaining = descend::<D::Error>(remaining, 1)?;
            let sequence = decoder.open_container(de::Kind::Seq, None).await?;
            Ok(Scalar::Tuple(
                scalar_items(decoder, sequence, remaining).await?,
            ))
        }
        de::Kind::Map => {
            let mut map = decoder.open_container(de::Kind::Map, None).await?;
            if map.slot() == de::Slot::End {
                descend::<D::Error>(remaining, 1)?;
                return Ok(Scalar::Map(Map::new()));
            }
            let key = first_key(decoder, &mut map).await?;
            if !key.starts_with('/') && !key.starts_with('$') {
                let remaining = descend::<D::Error>(remaining, 1)?;
                return Ok(Scalar::Map(
                    Box::pin(scalar_map(decoder, map, Some(key), remaining, true)).await?,
                ));
            }

            Box::pin(tagged_scalar(decoder, map, key, remaining)).await
        }
    }
}

// Keep tagged payload construction out of every ordinary container's polling
// frame. Boxing recursive futures alone does not bound their polling stack.
async fn tagged_scalar<D: de::Decoder>(
    decoder: &mut D,
    mut map: de::Container,
    key: String,
    remaining: usize,
) -> Result<Scalar, D::Error> {
    if let Some(value) = Box::pin(tc_value::decode_typed_value(
        &key, decoder, &mut map, remaining,
    ))
    .await?
    {
        finish_map(decoder, map, true).await?;
        return Ok(value.into());
    }

    let path = PathBuf::from_str(&key).ok();
    let value = if let Some(kind) = path.as_ref().and_then(OpDefType::from_path) {
        let remaining = descend::<D::Error>(remaining, 2)?;
        Scalar::Op(Box::pin(op_payload(decoder, kind, remaining)).await?)
    } else if path.as_deref().is_some_and(reference_path) {
        let remaining = descend::<D::Error>(remaining, 1)?;
        Box::pin(reference_payload(decoder, &key, remaining))
            .await?
            .into()
    } else {
        match Box::pin(args(decoder, remaining, 3)).await? {
            Scalar::Tuple(items) if items.is_empty() => {
                if key.starts_with('$') {
                    descend::<D::Error>(remaining, 1)?;
                    TCRef::Id(key.parse().map_err(de::Error::custom)?).into()
                } else {
                    Value::from(Link::from_str(&key).map_err(de::Error::custom)?).into()
                }
            }
            args => TCRef::Op(call::<D::Error>(&key, args)?).into(),
        }
    };

    decoder.finish_child(&mut map).await?;
    finish_map(decoder, map, !key.starts_with('$')).await?;
    Ok(value)
}

async fn reference<D: de::Decoder>(decoder: &mut D, remaining: usize) -> Result<TCRef, D::Error> {
    let mut map = decoder.open_container(de::Kind::Map, None).await?;
    let key = first_key(decoder, &mut map).await?;
    let value = Box::pin(reference_payload(decoder, &key, remaining)).await?;
    decoder.finish_child(&mut map).await?;
    finish_map(decoder, map, !key.starts_with('$')).await?;
    Ok(value)
}

async fn reference_payload<D: de::Decoder>(
    decoder: &mut D,
    key: &str,
    remaining: usize,
) -> Result<TCRef, D::Error> {
    let path = PathBuf::from_str(key).ok();
    let path = path.as_deref().unwrap_or_default();
    let control = path.len() == 4 && reference_path(path);
    if !control {
        if reference_path(path) {
            return Ok(TCRef::Op(
                Box::pin(call_payload(
                    decoder,
                    key,
                    descend::<D::Error>(remaining, 1)?,
                ))
                .await?,
            ));
        }
        return match Box::pin(args(decoder, remaining, 2)).await? {
            Scalar::Tuple(items) if items.is_empty() && key.starts_with('$') => {
                Ok(TCRef::Id(key.parse().map_err(de::Error::custom)?))
            }
            args => Ok(TCRef::Op(call::<D::Error>(key, args)?)),
        };
    }
    let remaining = descend::<D::Error>(remaining, 1)?;
    let mut sequence = decoder.open_container(de::Kind::Seq, None).await?;
    let value = if path == &crate::TCREF_COND[..] {
        require_field::<D::Error>(&sequence, 0, 3)?;
        let cond = Box::pin(reference(decoder, remaining)).await?;
        decoder.finish_child(&mut sequence).await?;
        let then = Box::pin(scalar_field(decoder, &mut sequence, remaining, 1, 3)).await?;
        let or_else = Box::pin(scalar_field(decoder, &mut sequence, remaining, 2, 3)).await?;
        TCRef::Cond(Box::new(Cond::new(cond, then, or_else)))
    } else if path == &crate::TCREF_AFTER[..] {
        let when = Box::pin(scalar_field(decoder, &mut sequence, remaining, 0, 2)).await?;
        let then = Box::pin(scalar_field(decoder, &mut sequence, remaining, 1, 2)).await?;
        TCRef::After(Box::new(After::new(when, then)))
    } else if path == &crate::TCREF_WHILE[..] {
        let cond = Box::pin(scalar_field(decoder, &mut sequence, remaining, 0, 3)).await?;
        let closure = Box::pin(scalar_field(decoder, &mut sequence, remaining, 1, 3)).await?;
        let state = Box::pin(scalar_field(decoder, &mut sequence, remaining, 2, 3)).await?;
        TCRef::While(Box::new(While::new(cond, closure, state)))
    } else {
        let items = Box::pin(scalar_field(decoder, &mut sequence, remaining, 0, 3)).await?;
        let op = Box::pin(scalar_field(decoder, &mut sequence, remaining, 1, 3)).await?;
        let name = Box::pin(scalar_field(decoder, &mut sequence, remaining, 2, 3)).await?;
        end_fields::<D::Error>(&sequence)?;
        let name = match name {
            Scalar::Value(Value::String(name)) => name.parse().map_err(de::Error::custom)?,
            _ => {
                return Err(de::Error::custom(
                    "invalid ForEach item_name (expected string)",
                ))
            }
        };
        TCRef::ForEach(Box::new(ForEach::new(items, op, name)))
    };

    end_fields::<D::Error>(&sequence)?;
    Ok(value)
}

async fn call_payload<D: de::Decoder>(
    decoder: &mut D,
    key: &str,
    remaining: usize,
) -> Result<OpRef, D::Error> {
    let path = PathBuf::from_str(key).ok();
    let path = path.as_deref().unwrap_or_default();
    if path.len() != 5 || !reference_path(path) {
        let args = Box::pin(args(decoder, remaining, 1)).await?;
        descend::<D::Error>(remaining, 1)?;
        return call::<D::Error>(key, args);
    }
    let remaining = descend::<D::Error>(remaining, 1)?;
    let mut sequence = decoder.open_container(de::Kind::Seq, None).await?;
    let put = path == &crate::OPREF_PUT[..];
    let count = if put { 3 } else { 2 };
    let subject = field::<Subject, _>(decoder, &mut sequence, 0, count).await?;
    let value = if path == &crate::OPREF_POST[..] {
        require_field::<D::Error>(&sequence, 1, count)?;
        let map = decoder.open_container(de::Kind::Map, None).await?;
        let params = Box::pin(scalar_map(decoder, map, None, remaining, true)).await?;
        decoder.finish_child(&mut sequence).await?;
        OpRef::Post((subject, params))
    } else {
        let key = Box::pin(scalar_field(decoder, &mut sequence, remaining, 1, count)).await?;
        if put {
            let value = Box::pin(scalar_field(decoder, &mut sequence, remaining, 2, count)).await?;
            OpRef::Put((subject, key, value))
        } else if path == &crate::OPREF_DELETE[..] {
            OpRef::Delete((subject, key))
        } else {
            OpRef::Get((subject, key))
        }
    };

    end_fields::<D::Error>(&sequence)?;
    Ok(value)
}

async fn form<D: de::Decoder>(
    decoder: &mut D,
    remaining: usize,
) -> Result<Vec<(Id, Scalar)>, D::Error> {
    let mut sequence = decoder.open_container(de::Kind::Seq, None).await?;
    let mut values = Vec::new();
    while sequence.slot() != de::Slot::End {
        let mut binding = decoder.open_container(de::Kind::Seq, None).await?;
        let id = field(decoder, &mut binding, 0, 2).await?;
        let value = Box::pin(scalar_field(decoder, &mut binding, remaining, 1, 2)).await?;
        if binding.slot() != de::Slot::End {
            return Err(de::Error::invalid_length(3, 2));
        }
        values.try_reserve(1).map_err(de::Error::custom)?;
        values.push((id, value));
        decoder.finish_child(&mut sequence).await?;
    }

    Ok(values)
}

async fn op_payload<D: de::Decoder>(
    decoder: &mut D,
    kind: OpDefType,
    remaining: usize,
) -> Result<OpDef, D::Error> {
    let op = if matches!(kind, OpDefType::Post) {
        OpDef::Post(Box::pin(form(decoder, remaining)).await?)
    } else {
        let mut sequence = decoder.open_container(de::Kind::Seq, None).await?;
        let count = if matches!(kind, OpDefType::Put) { 3 } else { 2 };
        let key = field(decoder, &mut sequence, 0, count).await?;
        let value = if matches!(kind, OpDefType::Put) {
            Some(field(decoder, &mut sequence, 1, count).await?)
        } else {
            None
        };
        require_field::<D::Error>(&sequence, count - 1, count)?;
        let form = Box::pin(form(decoder, remaining)).await?;
        decoder.finish_child(&mut sequence).await?;
        end_fields::<D::Error>(&sequence)?;
        match kind {
            OpDefType::Get => OpDef::Get((key, form)),
            OpDefType::Delete => OpDef::Delete((key, form)),
            OpDefType::Put => OpDef::Put((key, value.unwrap(), form)),
            OpDefType::Post => unreachable!(),
        }
    };

    op.validate().map_err(de::Error::custom)?;
    Ok(op)
}

impl FromStream for Scalar {
    type Context = ();

    async fn from_stream<D: de::Decoder>((): (), decoder: &mut D) -> Result<Self, D::Error> {
        decode_scalar(decoder, crate::MAX_SCALAR_DEPTH).await
    }
}

impl FromStream for TCRef {
    type Context = ();

    async fn from_stream<D: de::Decoder>((): (), decoder: &mut D) -> Result<Self, D::Error> {
        reference(decoder, crate::MAX_SCALAR_DEPTH).await
    }
}

impl FromStream for OpRef {
    type Context = ();

    async fn from_stream<D: de::Decoder>((): (), decoder: &mut D) -> Result<Self, D::Error> {
        let mut map = decoder.open_container(de::Kind::Map, None).await?;
        let key = first_key(decoder, &mut map).await?;
        let value = call_payload(decoder, &key, crate::MAX_SCALAR_DEPTH).await?;
        decoder.finish_child(&mut map).await?;
        finish_map(decoder, map, true).await?;
        Ok(value)
    }
}

impl FromStream for OpDef {
    type Context = ();

    async fn from_stream<D: de::Decoder>((): (), decoder: &mut D) -> Result<Self, D::Error> {
        let mut map = decoder.open_container(de::Kind::Map, None).await?;
        let key = first_key(decoder, &mut map).await?;
        let path = PathBuf::from_str(&key).ok();
        let kind = path
            .as_ref()
            .and_then(OpDefType::from_path)
            .ok_or_else(|| de::Error::custom("expected Op definition type"))?;
        let value = op_payload(
            decoder,
            kind,
            descend::<D::Error>(crate::MAX_SCALAR_DEPTH, 1)?,
        )
        .await?;
        decoder.finish_child(&mut map).await?;
        finish_map(decoder, map, true).await?;
        Ok(value)
    }
}

/// Decode a tagged operation payload using its enclosing owner's depth allowance.
/// Unknown paths leave the decoder untouched.
pub async fn decode_op_def_entry<D: de::Decoder>(
    path: &PathBuf,
    decoder: &mut D,
    map: &mut de::Container,
    remaining: usize,
) -> Result<Option<OpDef>, D::Error> {
    let Some(kind) = OpDefType::from_path(path) else {
        return Ok(None);
    };
    let remaining = descend::<D::Error>(remaining.min(crate::MAX_SCALAR_DEPTH), 1)?;
    let op = op_payload(decoder, kind, remaining).await?;
    decoder.finish_child(map).await?;
    Ok(Some(op))
}
