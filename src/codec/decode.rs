use std::collections::btree_map::Entry;
use std::str::FromStr;

use destream::de::{self, FromStream};
use pathlink::PathBuf;

use crate::op::OpDefType;
use crate::{After, Cond, ForEach, Id, Map, OpDef, OpRef, Scalar, Subject, TCRef, While};
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

// The child Scalar box breaks recursive future sizing; callers await this helper directly.
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

fn end_map<E: de::Error>(map: &de::Container) -> Result<(), E> {
    if map.slot() != de::Slot::End {
        return Err(E::custom("unexpected extra IR map entry"));
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

    Ok(values.into_boxed_slice().into_vec())
}

async fn scalar_map<D: de::Decoder>(
    decoder: &mut D,
    mut map: de::Container,
    mut key: Option<Id>,
    remaining: usize,
) -> Result<Map<Scalar>, D::Error> {
    let mut values = Map::new();
    while map.slot() != de::Slot::End {
        let id = match key.take() {
            Some(id) => id,
            None => first_key(decoder, &mut map)
                .await?
                .parse()
                .map_err(de::Error::custom)?,
        };

        let entry = match values.entry(id) {
            Entry::Vacant(entry) => entry,
            Entry::Occupied(entry) => {
                return Err(de::Error::custom(format!(
                    "duplicate map key {}",
                    entry.key()
                )));
            }
        };

        let value = Box::pin(decode_scalar(decoder, remaining)).await?;
        entry.insert(value);
        decoder.finish_child(&mut map).await?;
    }

    Ok(values)
}

// Subject syntax uses a sequence for GET/PUT and a map for POST.
// Empty sequences represent an Id reference only; links use URI literals.
async fn args<D: de::Decoder>(decoder: &mut D, remaining: usize) -> Result<Scalar, D::Error> {
    match decoder.peek_kind().await? {
        de::Kind::Seq => {
            let sequence = decoder.open_container(de::Kind::Seq, None).await?;
            if sequence.slot() == de::Slot::End {
                return Ok(Scalar::Tuple(Vec::new()));
            }
            let remaining = descend::<D::Error>(remaining, 3)?;
            Ok(Scalar::Tuple(
                scalar_items(decoder, sequence, remaining).await?,
            ))
        }
        de::Kind::Map => {
            let remaining = descend::<D::Error>(remaining, 3)?;
            let map = decoder.open_container(de::Kind::Map, None).await?;
            Ok(Scalar::Map(
                scalar_map(decoder, map, None, remaining).await?,
            ))
        }
        _ => Err(de::Error::custom("invalid IR container shape")),
    }
}

fn call<E: de::Error>(key: &str, args: Scalar) -> Result<OpRef, E> {
    let subject = key.parse::<Subject>().map_err(E::custom)?;
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
            // Ordinary maps have identifier keys; tags and subjects use their
            // own parsers below. Reuse the parsed identifier in the map.
            if let Ok(id) = key.parse::<Id>() {
                let remaining = descend::<D::Error>(remaining, 1)?;
                return Ok(Scalar::Map(
                    Box::pin(scalar_map(decoder, map, Some(id), remaining)).await?,
                ));
            }

            Box::pin(decode_scalar_entry(decoder, map, key, remaining)).await
        }
    }
}

/// Decode a scalar map entry after its owner consumed the first key and colon.
/// Recognized type paths take precedence over invocation subjects. This consumes
/// the payload and requires the map to end immediately afterward.
pub async fn decode_scalar_entry<D: de::Decoder>(
    decoder: &mut D,
    mut map: de::Container,
    key: String,
    remaining: usize,
) -> Result<Scalar, D::Error> {
    let remaining = remaining.min(crate::MAX_SCALAR_DEPTH);
    let value = Box::pin(entry_value(decoder, &mut map, &key, remaining)).await?;
    end_map::<D::Error>(&map)?;
    Ok(value)
}

async fn entry_value<D: de::Decoder>(
    decoder: &mut D,
    map: &mut de::Container,
    key: &str,
    remaining: usize,
) -> Result<Scalar, D::Error> {
    let path = PathBuf::from_str(key).ok();
    let value = match &path {
        Some(path) => Box::pin(tc_value::decode_typed_value(path, decoder, map, remaining)).await?,
        None => None,
    };

    if let Some(value) = value {
        return Ok(value.into());
    }

    let value = if let Some(kind) = path.as_ref().and_then(OpDefType::from_path) {
        let remaining = descend::<D::Error>(remaining, 2)?;
        Scalar::Op(Box::pin(op_payload(decoder, kind, remaining)).await?)
    } else if let Some(path) = path.as_deref().filter(|path| reference_path(path)) {
        let remaining = descend::<D::Error>(remaining, 1)?;
        Box::pin(reference_payload(decoder, path, remaining))
            .await?
            .into()
    } else {
        match Box::pin(args(decoder, remaining)).await? {
            Scalar::Tuple(items) if items.is_empty() => {
                if key.starts_with('$') {
                    descend::<D::Error>(remaining, 1)?;
                    TCRef::Id(key.parse().map_err(de::Error::custom)?).into()
                } else {
                    return Err(de::Error::custom("literal links must use URI text"));
                }
            }
            args => TCRef::Op(call::<D::Error>(key, args)?).into(),
        }
    };

    decoder.finish_child(map).await?;
    Ok(value)
}

async fn reference<D: de::Decoder>(decoder: &mut D, remaining: usize) -> Result<TCRef, D::Error> {
    let mut map = decoder.open_container(de::Kind::Map, None).await?;
    let key = first_key(decoder, &mut map).await?;
    // A directly typed reference already owns its outer envelope. Preserve its
    // allowance while sharing Scalar's payload interpretation and precedence.
    let value = Box::pin(entry_value(decoder, &mut map, &key, remaining + 1)).await?;
    end_map::<D::Error>(&map)?;
    match value {
        Scalar::Ref(value) => Ok(*value),
        _ => Err(de::Error::custom("expected a reference")),
    }
}

async fn reference_payload<D: de::Decoder>(
    decoder: &mut D,
    path: &[Id],
    remaining: usize,
) -> Result<TCRef, D::Error> {
    // The entry decoder has already recognized this exact reference path.
    if path.len() == 5 {
        return Ok(TCRef::Op(
            Box::pin(call_payload(
                decoder,
                path,
                descend::<D::Error>(remaining, 1)?,
            ))
            .await?,
        ));
    }

    let remaining = descend::<D::Error>(remaining, 1)?;
    let mut sequence = decoder.open_container(de::Kind::Seq, None).await?;
    let value = if path == &crate::TCREF_COND[..] {
        require_field::<D::Error>(&sequence, 0, 3)?;
        let cond = Box::pin(reference(decoder, remaining)).await?;
        decoder.finish_child(&mut sequence).await?;
        let then = scalar_field(decoder, &mut sequence, remaining, 1, 3).await?;
        let or_else = scalar_field(decoder, &mut sequence, remaining, 2, 3).await?;
        TCRef::Cond(Box::new(Cond::new(cond, then, or_else)))
    } else if path == &crate::TCREF_AFTER[..] {
        let when = scalar_field(decoder, &mut sequence, remaining, 0, 2).await?;
        let then = scalar_field(decoder, &mut sequence, remaining, 1, 2).await?;
        TCRef::After(Box::new(After::new(when, then)))
    } else if path == &crate::TCREF_WHILE[..] {
        let cond = scalar_field(decoder, &mut sequence, remaining, 0, 3).await?;
        let closure = scalar_field(decoder, &mut sequence, remaining, 1, 3).await?;
        let state = scalar_field(decoder, &mut sequence, remaining, 2, 3).await?;
        TCRef::While(Box::new(While::new(cond, closure, state)))
    } else {
        let items = scalar_field(decoder, &mut sequence, remaining, 0, 3).await?;
        let op = scalar_field(decoder, &mut sequence, remaining, 1, 3).await?;
        let name = scalar_field(decoder, &mut sequence, remaining, 2, 3).await?;
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
    path: &[Id],
    remaining: usize,
) -> Result<OpRef, D::Error> {
    let remaining = descend::<D::Error>(remaining, 1)?;
    let mut sequence = decoder.open_container(de::Kind::Seq, None).await?;
    let put = path == &crate::OPREF_PUT[..];
    let count = if put { 3 } else { 2 };
    let subject = field::<Subject, _>(decoder, &mut sequence, 0, count).await?;
    let value = if path == &crate::OPREF_POST[..] {
        require_field::<D::Error>(&sequence, 1, count)?;
        let map = decoder.open_container(de::Kind::Map, None).await?;
        let params = Box::pin(scalar_map(decoder, map, None, remaining)).await?;
        decoder.finish_child(&mut sequence).await?;
        OpRef::Post((subject, params))
    } else {
        let key = scalar_field(decoder, &mut sequence, remaining, 1, count).await?;
        if put {
            let value = scalar_field(decoder, &mut sequence, remaining, 2, count).await?;
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
        let value = scalar_field(decoder, &mut binding, remaining, 1, 2).await?;
        if binding.slot() != de::Slot::End {
            return Err(de::Error::invalid_length(3, 2));
        }
        values.try_reserve(1).map_err(de::Error::custom)?;
        values.push((id, value));
        decoder.finish_child(&mut sequence).await?;
    }

    Ok(values.into_boxed_slice().into_vec())
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
        // OpRef owns both the reference and call envelopes.
        let value = Box::pin(entry_value(
            decoder,
            &mut map,
            &key,
            crate::MAX_SCALAR_DEPTH + 2,
        ))
        .await?;

        end_map::<D::Error>(&map)?;
        match value {
            Scalar::Ref(value) => match *value {
                TCRef::Op(value) => Ok(value),
                _ => Err(de::Error::custom("expected an operation reference")),
            },
            _ => Err(de::Error::custom("expected an operation reference")),
        }
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
        end_map::<D::Error>(&map)?;
        Ok(value)
    }
}
