use std::collections::BTreeSet;
use std::str::FromStr;

use crate::{Id, Map, Method, Scalar, Subject};
use async_hash::{Digest, Hash, Output};
use destream::{de, en, EncodeMap, IntoStream};
use pathlink::PathBuf;
use tc_error::{TCError, TCResult};

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
pub(crate) enum OpDefType {
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
    pub fn form(&self) -> &[(Id, Scalar)] {
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

    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        crate::scalar::collect_op_requires(self, required)
    }

    /// Validate that this operation is a single-assignment lexical scope.
    pub fn validate(&self) -> TCResult<()> {
        crate::scalar::validate_op(self, &BTreeSet::new())
    }
}

pub(crate) fn bind(
    scope: &mut BTreeSet<Id>,
    local: &mut BTreeSet<Id>,
    id: &Id,
    role: &str,
) -> TCResult<()> {
    if id.as_str() == "self" {
        return Err(TCError::bad_request(format!(
            "cannot bind reserved identifier $self as an OpDef {role}"
        )));
    }

    if scope.contains(id) {
        let collision = if local.contains(id) {
            "duplicate"
        } else {
            "shadowed"
        };
        return Err(TCError::bad_request(format!(
            "{collision} OpDef binding ${id}"
        )));
    }

    local.insert(id.clone());
    scope.insert(id.clone());
    Ok(())
}

impl OpRef {
    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        crate::scalar::collect_op_ref_requires(self, required)
    }

    pub(crate) fn visit_referenced_methods(
        &self,
        visitor: &mut impl FnMut(&pathlink::Link, Method),
    ) {
        let (method, subject) = match self {
            Self::Get((subject, key)) => {
                key.visit_referenced_methods(visitor);
                (Method::Get, subject)
            }
            Self::Put((subject, key, value)) => {
                key.visit_referenced_methods(visitor);
                value.visit_referenced_methods(visitor);
                (Method::Put, subject)
            }
            Self::Post((subject, params)) => {
                for scalar in params.values() {
                    scalar.visit_referenced_methods(visitor);
                }
                (Method::Post, subject)
            }
            Self::Delete((subject, key)) => {
                key.visit_referenced_methods(visitor);
                (Method::Delete, subject)
            }
        };
        if let Subject::Link(link) = subject {
            visitor(link, method);
        }
    }
}

impl<D: Digest> Hash<D> for &OpRef {
    fn hash(self) -> Output<D> {
        match self {
            OpRef::Get((subject, key)) | OpRef::Delete((subject, key)) => {
                Hash::<D>::hash((subject, key))
            }
            OpRef::Put((subject, key, value)) => Hash::<D>::hash((subject, key, value)),
            OpRef::Post((subject, params)) => Hash::<D>::hash((subject, params)),
        }
    }
}

impl<D: Digest> Hash<D> for &OpDef {
    fn hash(self) -> Output<D> {
        match self {
            OpDef::Get((key, form)) | OpDef::Delete((key, form)) => Hash::<D>::hash((key, form)),
            OpDef::Put((key, value, form)) => Hash::<D>::hash((key, value, form)),
            OpDef::Post(form) => Hash::<D>::hash(form),
        }
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
                let path =
                    PathBuf::from_str(&key).map_err(|err| de::Error::custom(err.to_string()))?;
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

    op.validate()
        .map_err(|err| de::Error::custom(err.to_string()))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdRef, TCRef};
    use bytes::Bytes;
    use futures::{future, stream};

    fn id(name: &str) -> Id {
        name.parse().expect("Id")
    }

    fn reference(name: &str) -> Scalar {
        Scalar::from(TCRef::Id(name.parse::<IdRef>().expect("IdRef")))
    }

    #[test]
    fn requires_reports_only_unbound_lexical_ids() {
        let op = OpDef::Get((
            id("key"),
            vec![
                (id("local"), reference("$input")),
                (id("result"), reference("$local")),
            ],
        ));
        let mut required = BTreeSet::new();
        op.requires(&mut required);

        assert_eq!(required, BTreeSet::from([id("input")]));
    }

    #[test]
    fn validation_enforces_single_assignment() {
        assert!(OpDef::Post(Vec::new()).validate().is_err());

        let duplicate = OpDef::Post(vec![
            (id("result"), Scalar::from(1_u64)),
            (id("result"), Scalar::from(2_u64)),
        ]);
        assert!(duplicate.validate().is_err());

        let shadows_parameter = OpDef::Get((id("key"), vec![(id("key"), Scalar::from(1_u64))]));
        assert!(shadows_parameter.validate().is_err());

        let duplicate_parameters = OpDef::Put((
            id("value"),
            id("value"),
            vec![(id("result"), Scalar::default())],
        ));
        assert!(duplicate_parameters.validate().is_err());

        let reserved = OpDef::Post(vec![(id("self"), Scalar::from(1_u64))]);
        assert!(reserved.validate().is_err());
    }

    #[test]
    fn validation_rejects_nested_shadowing_and_control_collisions() {
        let nested_shadow = OpDef::Post(vec![
            (id("outer"), Scalar::from(1_u64)),
            (
                id("result"),
                Scalar::Op(OpDef::Post(vec![(id("outer"), Scalar::from(2_u64))])),
            ),
        ]);
        assert!(nested_shadow.validate().is_err());

        let foreach_shadow = OpDef::Post(vec![
            (id("item"), Scalar::from(1_u64)),
            (
                id("result"),
                Scalar::from(TCRef::ForEach(Box::new(crate::ForEach::new(
                    Scalar::Tuple(Vec::new()),
                    Scalar::Op(OpDef::Post(vec![(id("each"), reference("$item"))])),
                    id("item"),
                )))),
            ),
        ]);
        assert!(foreach_shadow.validate().is_err());

        let while_shadow = OpDef::Post(vec![
            (id("state"), Scalar::from(1_u64)),
            (
                id("result"),
                Scalar::from(TCRef::While(Box::new(crate::While::new(
                    Scalar::Op(OpDef::Post(vec![(id("condition"), Scalar::from(1_u64))])),
                    Scalar::Op(OpDef::Post(vec![(id("next"), Scalar::from(0_u64))])),
                    Scalar::from(0_u64),
                )))),
            ),
        ]);
        assert!(while_shadow.validate().is_err());
    }

    #[test]
    fn sibling_scopes_may_reuse_private_names() {
        let branch = || Scalar::Op(OpDef::Post(vec![(id("private"), Scalar::from(1_u64))]));
        let op = OpDef::Post(vec![(
            id("result"),
            Scalar::Tuple(vec![branch(), branch()]),
        )]);

        op.validate().expect("disjoint sibling scopes");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decoding_rejects_duplicate_bindings() {
        let duplicate = OpDef::Post(vec![
            (id("result"), Scalar::from(1_u64)),
            (id("result"), Scalar::from(2_u64)),
        ]);
        let encoded = destream_json::encode(duplicate).expect("encode invalid OpDef");
        let decoded: Result<OpDef, _> = destream_json::try_decode((), encoded).await;
        let error = decoded.expect_err("reject duplicate binding");

        assert!(error.to_string().contains("duplicate OpDef binding"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decoding_rejects_an_empty_operation() {
        let encoded = destream_json::encode(OpDef::Post(Vec::new())).expect("encode empty OpDef");
        let decoded: Result<OpDef, _> = destream_json::try_decode((), encoded).await;
        let error = decoded.expect_err("reject empty operation");

        assert!(error.to_string().contains("at least one provider"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shared_lexical_fixtures_enforce_the_contract() {
        for fixture in [
            include_str!("../fixtures/lexical/valid-forward.json"),
            include_str!("../fixtures/lexical/valid-capture.json"),
        ] {
            let source = stream::once(future::ready(Ok::<_, std::convert::Infallible>(
                Bytes::copy_from_slice(fixture.as_bytes()),
            )));
            let op: OpDef = destream_json::try_decode((), source)
                .await
                .expect("valid lexical fixture");
            op.validate().expect("valid lexical scope");
        }

        for fixture in [
            include_str!("../fixtures/lexical/invalid-shadow.json"),
            include_str!("../fixtures/lexical/invalid-foreach.json"),
            include_str!("../fixtures/lexical/invalid-while.json"),
        ] {
            let source = stream::once(future::ready(Ok::<_, std::convert::Infallible>(
                Bytes::copy_from_slice(fixture.as_bytes()),
            )));
            let decoded: Result<OpDef, _> = destream_json::try_decode((), source).await;
            decoded.expect_err("invalid lexical fixture");
        }
    }
}
