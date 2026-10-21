use destream::en::{self, EncodeMap, IntoStream, MapStream, SeqStream, ToStream};
use futures::stream;

use crate::{OpDef, OpRef, Scalar, TCRef};

fn tagged<'en, E: en::Encoder<'en>, T: IntoStream<'en> + 'en>(
    encoder: E,
    key: impl ToString,
    value: T,
) -> Result<E::Ok, E::Error> {
    let mut map = encoder.encode_map(Some(1))?;
    map.encode_entry(key.to_string(), value)?;
    map.end()
}

impl<'en> IntoStream<'en> for Scalar {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Value(value) => value.into_stream(encoder),
            Self::Ref(value) => (*value).into_stream(encoder),
            Self::Op(value) => value.into_stream(encoder),
            Self::Map(values) => encoder.encode_map_stream(stream::iter(values)),
            Self::Tuple(values) => encoder.encode_seq_stream(stream::iter(values)),
        }
    }
}

impl<'en> ToStream<'en> for Scalar {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Value(value) => value.to_stream(encoder),
            Self::Ref(value) => value.to_stream(encoder),
            Self::Op(value) => value.to_stream(encoder),
            Self::Map(values) => encoder.encode_map_stream(stream::iter(values)),
            Self::Tuple(values) => encoder.encode_seq_stream(stream::iter(values)),
        }
    }
}

impl<'en> IntoStream<'en> for TCRef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Op(value) => value.into_stream(encoder),
            Self::Id(value) => tagged(encoder, value, SeqStream::from(stream::empty::<Scalar>())),
            Self::Cond(value) => {
                let crate::Cond {
                    cond,
                    then,
                    or_else,
                } = *value;
                tagged(encoder, crate::TCREF_COND, (cond, then, or_else))
            }
            Self::After(value) => {
                let crate::After { when, then } = *value;
                tagged(encoder, crate::TCREF_AFTER, (when, then))
            }
            Self::While(value) => {
                let crate::While {
                    cond,
                    closure,
                    state,
                } = *value;
                tagged(encoder, crate::TCREF_WHILE, (cond, closure, state))
            }
            Self::ForEach(value) => {
                let crate::ForEach {
                    items,
                    op,
                    item_name,
                } = *value;
                tagged(encoder, crate::TCREF_FOR_EACH, (items, op, item_name))
            }
        }
    }
}

impl<'en> ToStream<'en> for TCRef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Op(value) => value.to_stream(encoder),
            Self::Id(value) => tagged(encoder, value, SeqStream::from(stream::empty::<Scalar>())),
            Self::Cond(value) => tagged(
                encoder,
                crate::TCREF_COND,
                (&value.cond, &value.then, &value.or_else),
            ),
            Self::After(value) => tagged(encoder, crate::TCREF_AFTER, (&value.when, &value.then)),
            Self::While(value) => tagged(
                encoder,
                crate::TCREF_WHILE,
                (&value.cond, &value.closure, &value.state),
            ),
            Self::ForEach(value) => tagged(
                encoder,
                crate::TCREF_FOR_EACH,
                (&value.items, &value.op, &value.item_name),
            ),
        }
    }
}

impl<'en> IntoStream<'en> for OpRef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Get((subject, key)) => tagged(encoder, subject, (key,)),
            Self::Put((subject, key, value)) => tagged(encoder, subject, (key, value)),
            Self::Post((subject, params)) => {
                tagged(encoder, subject, MapStream::from(stream::iter(params)))
            }
            Self::Delete((subject, key)) => {
                tagged(encoder, crate::OPREF_DELETE, (subject.to_string(), key))
            }
        }
    }
}

impl<'en> ToStream<'en> for OpRef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Get((subject, key)) => tagged(encoder, subject, (key,)),
            Self::Put((subject, key, value)) => tagged(encoder, subject, (key, value)),
            Self::Post((subject, params)) => {
                tagged(encoder, subject, MapStream::from(stream::iter(params)))
            }
            Self::Delete((subject, key)) => {
                tagged(encoder, crate::OPREF_DELETE, (subject.to_string(), key))
            }
        }
    }
}

impl<'en> IntoStream<'en> for OpDef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let tag = self.class().path();
        match self {
            Self::Get((key, form)) | Self::Delete((key, form)) => {
                tagged(encoder, tag, (key, SeqStream::from(stream::iter(form))))
            }
            Self::Put((key, value, form)) => tagged(
                encoder,
                tag,
                (key, value, SeqStream::from(stream::iter(form))),
            ),
            Self::Post(form) => tagged(encoder, tag, SeqStream::from(stream::iter(form))),
        }
    }
}

impl<'en> ToStream<'en> for OpDef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        let tag = self.class().path();
        match self {
            Self::Get((key, form)) | Self::Delete((key, form)) => {
                tagged(encoder, tag, (key, SeqStream::from(stream::iter(form))))
            }
            Self::Put((key, value, form)) => tagged(
                encoder,
                tag,
                (key, value, SeqStream::from(stream::iter(form))),
            ),
            Self::Post(form) => tagged(encoder, tag, SeqStream::from(stream::iter(form))),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/codec/encode.rs"]
mod tests;
