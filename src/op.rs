use std::str::FromStr;

use destream::{de, en, EncodeMap, IntoStream};
use pathlink::PathBuf;
use crate::{Id, Map, Scalar, Subject};

/// The data defining a reference to a GET op.
pub type GetRef = (Subject, Scalar);

/// The data defining a reference to a PUT op.
pub type PutRef = (Subject, Scalar, Scalar);

/// The data defining a reference to a POST op.
pub type PostRef = (Subject, Map<Scalar>);

/// The data defining a reference to a DELETE op.
pub type DeleteRef = (Subject, Scalar);

/// A reference to an op.
///
/// This is a structural port of the v1 `OpRef` enum. Resolution/execution is implemented by the
/// host kernel and is intentionally not part of this type definition.
///
/// ## v1-compatible JSON semantics
///
/// Encoded as a single-entry map:
///
/// - GET: `{ "<subject>": [<key>] }`
/// - PUT: `{ "<subject>": [<key>, <value>] }`
/// - POST: `{ "<subject>": { "<name>": <value>, ... } }`
/// - DELETE: `{ "/state/scalar/ref/op/delete": [<subject>, <key>] }`
#[derive(Clone, Debug, PartialEq)]
pub enum OpRef {
    Get(GetRef),
    Put(PutRef),
    Post(PostRef),
    Delete(DeleteRef),
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpDefType {
    Get,
    Put,
    Post,
    Delete,
}

impl OpDefType {
    pub(crate) fn from_path(path: &PathBuf) -> Option<Self> {
        let segments = path.as_ref();
        if segments.len() != 4 {
            return None;
        }

        if segments[..3] != crate::OPDEF_PREFIX[..] {
            return None;
        }

        match segments[3].as_str() {
            "get" => Some(Self::Get),
            "put" => Some(Self::Put),
            "post" => Some(Self::Post),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }

    fn path(&self) -> PathBuf {
        match self {
            Self::Get => PathBuf::from(crate::OPDEF_GET),
            Self::Put => PathBuf::from(crate::OPDEF_PUT),
            Self::Post => PathBuf::from(crate::OPDEF_POST),
            Self::Delete => PathBuf::from(crate::OPDEF_DELETE),
        }
    }
}

pub type GetOp = (Id, Vec<(Id, Scalar)>);
pub type PutOp = (Id, Id, Vec<(Id, Scalar)>);
pub type PostOp = Vec<(Id, Scalar)>;
pub type DeleteOp = (Id, Vec<(Id, Scalar)>);

#[derive(Clone, Debug, PartialEq)]
pub enum OpDef {
    Get(GetOp),
    Put(PutOp),
    Post(PostOp),
    Delete(DeleteOp),
}

impl OpDef {
    pub fn form(&self) -> &Vec<(Id, Scalar)> {
        match self {
            Self::Get((_, form)) => form,
            Self::Put((_, _, form)) => form,
            Self::Post(form) => form,
            Self::Delete((_, form)) => form,
        }
    }

    pub fn last_id(&self) -> Option<&Id> {
        self.form().last().map(|(id, _)| id)
    }

    fn class(&self) -> OpDefType {
        match self {
            Self::Get(_) => OpDefType::Get,
            Self::Put(_) => OpDefType::Put,
            Self::Post(_) => OpDefType::Post,
            Self::Delete(_) => OpDefType::Delete,
        }
    }

    pub fn walk_scalars(&self) -> OpDefScalarWalk<'_> {
        OpDefScalarWalk::new(self)
    }
}

impl de::FromStream for OpDef {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct OpDefVisitor;

        impl de::Visitor for OpDefVisitor {
            type Value = OpDef;

            fn expecting() -> &'static str {
                "an Op definition"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let key = map
                    .next_key::<String>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("expected Op definition type"))?;
                let path = PathBuf::from_str(&key)
                    .map_err(|err| de::Error::custom(err.to_string()))?;
                let op_def_type = OpDefType::from_path(&path).ok_or_else(|| {
                    de::Error::custom("expected Op definition type, e.g. \"/state/scalar/op/get\"")
                })?;

                decode_opdef_map_entry(op_def_type, &mut map).await
            }
        }

        decoder.decode_map(OpDefVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for OpDef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeMap;

        let mut map = encoder.encode_map(Some(1))?;
        let class = self.class().path().to_string();
        match self {
            Self::Get(def) => map.encode_entry(class, def)?,
            Self::Put(def) => map.encode_entry(class, def)?,
            Self::Post(def) => map.encode_entry(class, def)?,
            Self::Delete(def) => map.encode_entry(class, def)?,
        }
        map.end()
    }
}

impl<'en> en::ToStream<'en> for OpDef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

impl de::FromStream for OpRef {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct OpRefVisitor;

        impl de::Visitor for OpRefVisitor {
            type Value = OpRef;

            fn expecting() -> &'static str {
                "an OpRef map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let key = map
                    .next_key::<String>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("expected OpRef, found empty map"))?;

                decode_opref_map_entry(key, &mut map).await
            }
        }

        decoder.decode_map(OpRefVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for OpRef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            OpRef::Get((subject, key)) => {
                let mut map = encoder.encode_map(Some(1))?;
                map.encode_key(subject.to_string())?;
                map.encode_value(ScalarSeq::new(vec![key]))?;
                map.end()
            }
            OpRef::Put((subject, key, value)) => {
                let mut map = encoder.encode_map(Some(1))?;
                map.encode_key(subject.to_string())?;
                map.encode_value(ScalarSeq::new(vec![key, value]))?;
                map.end()
            }
            OpRef::Post((subject, params)) => {
                let mut map = encoder.encode_map(Some(1))?;
                map.encode_entry(subject.to_string(), params)?;
                map.end()
            }
            OpRef::Delete((subject, key)) => {
                let mut map = encoder.encode_map(Some(1))?;
                map.encode_key(PathBuf::from(crate::OPREF_DELETE).to_string())?;
                map.encode_value(SubjectScalarSeq::new(subject, key))?;
                map.end()
            }
        }
    }
}

impl<'en> en::ToStream<'en> for OpRef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
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

struct SubjectScalarSeq {
    subject: Subject,
    key: Scalar,
}

impl SubjectScalarSeq {
    fn new(subject: Subject, key: Scalar) -> Self {
        Self { subject, key }
    }
}

impl<'en> en::IntoStream<'en> for SubjectScalarSeq {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeSeq;

        let mut seq = encoder.encode_seq(Some(2))?;
        seq.encode_element(self.subject)?;
        seq.encode_element(self.key)?;
        seq.end()
    }
}

/// Internal helper used to decode `OpRef` and `TCRef` argument shapes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OpArgs {
    Map(Map<Scalar>),
    Seq(Vec<Scalar>),
}

impl de::FromStream for OpArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct ArgsVisitor;

        impl de::Visitor for ArgsVisitor {
            type Value = OpArgs;

            fn expecting() -> &'static str {
                "OpRef args (a sequence or map)"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut params = Map::<Scalar>::new();
                while let Some(key) = map.next_key::<Id>(()).await? {
                    let value = map.next_value::<Scalar>(()).await?;
                    params.insert(key, value);
                }
                Ok(OpArgs::Map(params))
            }

            async fn visit_seq<A: de::SeqAccess>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut items = if let Some(len) = access.size_hint() {
                    Vec::with_capacity(len)
                } else {
                    Vec::new()
                };

                while let Some(item) = access.next_element::<Scalar>(()).await? {
                    items.push(item);
                }

                Ok(OpArgs::Seq(items))
            }
        }

        decoder.decode_any(ArgsVisitor).await
    }
}

pub(crate) async fn decode_opdef_map_entry<A: de::MapAccess>(
    op_def_type: OpDefType,
    map: &mut A,
) -> Result<OpDef, A::Error> {
    let op = match op_def_type {
        OpDefType::Get => OpDef::Get(map.next_value::<GetOp>(()).await?),
        OpDefType::Put => OpDef::Put(map.next_value::<PutOp>(()).await?),
        OpDefType::Post => OpDef::Post(map.next_value::<PostOp>(()).await?),
        OpDefType::Delete => OpDef::Delete(map.next_value::<DeleteOp>(()).await?),
    };

    while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
        let _ = map.next_value::<de::IgnoredAny>(()).await?;
    }

    Ok(op)
}

pub(crate) fn opref_from_subject_args<E: de::Error>(
    subject: Subject,
    args: OpArgs,
) -> Result<OpRef, E> {
    match args {
        OpArgs::Map(params) => Ok(OpRef::Post((subject, params))),
        OpArgs::Seq(items) => match items.as_slice() {
            [key] => Ok(OpRef::Get((subject, key.clone()))),
            [key, value] => Ok(OpRef::Put((subject, key.clone(), value.clone()))),
            _ => Err(de::Error::custom(
                "invalid OpRef params (expected 1 or 2 elements)",
            )),
        },
    }
}

pub(crate) async fn decode_opref_map_entry<A: de::MapAccess>(
    key: String,
    map: &mut A,
) -> Result<OpRef, A::Error> {
    let op = if key.starts_with('/') {
        let path = PathBuf::from_str(&key).ok();

        if path.as_ref() == Some(&PathBuf::from(crate::OPREF_GET)) {
            let get = map.next_value::<(Subject, Scalar)>(()).await?;
            OpRef::Get(get)
        } else if path.as_ref() == Some(&PathBuf::from(crate::OPREF_PUT)) {
            let put = map.next_value::<(Subject, Scalar, Scalar)>(()).await?;
            OpRef::Put(put)
        } else if path.as_ref() == Some(&PathBuf::from(crate::OPREF_POST)) {
            let post = map.next_value::<(Subject, Map<Scalar>)>(()).await?;
            OpRef::Post(post)
        } else if path.as_ref() == Some(&PathBuf::from(crate::OPREF_DELETE)) {
            let delete = map.next_value::<(Subject, Scalar)>(()).await?;
            OpRef::Delete(delete)
        } else {
            let subject = crate::scalar::subject_from_str(&key)
                .map_err(|err| de::Error::custom(err.to_string()))?;

            let args = map.next_value::<OpArgs>(()).await?;
            match args {
                OpArgs::Map(params) => OpRef::Post((subject, params)),
                OpArgs::Seq(items) => match items.as_slice() {
                    [key] => OpRef::Get((subject, key.clone())),
                    [key, value] => OpRef::Put((subject, key.clone(), value.clone())),
                    _ => {
                        return Err(de::Error::custom(
                            "invalid OpRef params (expected 1 or 2 elements)",
                        ));
                    }
                },
            }
        }
    } else {
        let subject = crate::scalar::subject_from_str(&key)
            .map_err(|err| de::Error::custom(err.to_string()))?;

        let args = map.next_value::<OpArgs>(()).await?;
        match args {
            OpArgs::Map(params) => OpRef::Post((subject, params)),
            OpArgs::Seq(items) => match items.as_slice() {
                [key] => OpRef::Get((subject, key.clone())),
                [key, value] => OpRef::Put((subject, key.clone(), value.clone())),
                _ => {
                    return Err(de::Error::custom(
                        "invalid OpRef params (expected 1 or 2 elements)",
                    ));
                }
            },
        }
    };

    while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
        let _ = map.next_value::<de::IgnoredAny>(()).await?;
    }

    Ok(op)
}

pub struct OpDefScalarWalk<'a> {
    form_iter: std::slice::Iter<'a, (Id, Scalar)>,
    current: Option<crate::scalar::ScalarWalk<'a>>,
}

impl<'a> OpDefScalarWalk<'a> {
    fn new(op: &'a OpDef) -> Self {
        Self {
            form_iter: op.form().iter(),
            current: None,
        }
    }
}

impl<'a> Iterator for OpDefScalarWalk<'a> {
    type Item = &'a Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = &mut self.current {
                if let Some(item) = current.next() {
                    return Some(item);
                }
            }

            let (_, scalar) = self.form_iter.next()?;
            self.current = Some(crate::scalar::ScalarWalk::new(scalar));
        }
    }
}
