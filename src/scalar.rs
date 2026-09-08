use std::{fmt, str::FromStr};

use destream::{de, en, IntoStream};
use number_general::Number;
use pathlink::{path_label, Link, PathBuf, PathLabel};
use tc_error::TCError;
use tc_value::{decode_typed_value_map_entry, Value};

use crate::{Id, Map};

/// Scalar values exchanged via the TinyChain IR.
///
/// ## v1-compatible JSON semantics
///
/// This crate intentionally uses the v1 TinyChain reference encoding conventions when serialized
/// via `destream_json`:
///
/// - A scalar value is encoded like a v1 scalar value (e.g. `null`, or a typed map like
///   `{"\/state\/scalar\/value\/number": 3}`).
/// - A reference is encoded as an op ref / TC ref map (see [`crate::OpRef`] and [`crate::TCRef`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Value(Value),
    Ref(Box<crate::tcref::TCRef>),
    Op(crate::op::OpDef),
    Map(Map<Scalar>),
    Tuple(Vec<Scalar>),
    /// A typed application member embedded in another literal definition.
    Application(Box<crate::ApplicationDefinition<Scalar>>),
}

/// One node in the canonical recursive Scalar/reference traversal.
#[derive(Clone, Copy)]
pub enum ScalarNode<'a> {
    Scalar(&'a Scalar),
    Ref(&'a crate::TCRef),
    Op(&'a crate::OpRef),
    Subject(&'a crate::Subject),
    Binding(&'a Id),
}

impl Scalar {
    /// Visit recursive IR structure once, delegating meaning to the caller.
    pub fn visit<'a>(&'a self, visitor: &mut impl FnMut(ScalarNode<'a>) -> bool) {
        fn op<'a>(op: &'a crate::OpRef, visit: &mut impl FnMut(ScalarNode<'a>) -> bool) {
            let _ = visit(ScalarNode::Op(op));
            match op {
                crate::OpRef::Get((subject, key)) | crate::OpRef::Delete((subject, key)) => {
                    let _ = visit(ScalarNode::Subject(subject));
                    key.visit(visit);
                }
                crate::OpRef::Put((subject, key, value)) => {
                    let _ = visit(ScalarNode::Subject(subject));
                    key.visit(visit);
                    value.visit(visit);
                }
                crate::OpRef::Post((subject, params)) => {
                    let _ = visit(ScalarNode::Subject(subject));
                    for (name, scalar) in params {
                        let _ = visit(ScalarNode::Binding(name));
                        scalar.visit(visit);
                    }
                }
            }
        }

        fn walk_ref<'a>(
            reference: &'a crate::TCRef,
            visit: &mut impl FnMut(ScalarNode<'a>) -> bool,
        ) {
            let _ = visit(ScalarNode::Ref(reference));
            match reference {
                crate::TCRef::Id(_) => {}
                crate::TCRef::Op(operation) => op(operation, visit),
                crate::TCRef::Cond(cond) => {
                    walk_ref(&cond.cond, visit);
                    cond.then.visit(visit);
                    cond.or_else.visit(visit);
                }
                crate::TCRef::After(after) => {
                    after.when.visit(visit);
                    after.then.visit(visit);
                }
                crate::TCRef::While(while_ref) => {
                    while_ref.cond.visit(visit);
                    while_ref.closure.visit(visit);
                    while_ref.state.visit(visit);
                }
                crate::TCRef::ForEach(for_each) => {
                    for_each.items.visit(visit);
                    for_each.op.visit(visit);
                    let _ = visit(ScalarNode::Binding(&for_each.item_name));
                }
            }
        }

        if !visitor(ScalarNode::Scalar(self)) {
            return;
        }
        match self {
            Self::Value(_) => {}
            Self::Ref(reference_value) => walk_ref(reference_value, visitor),
            Self::Op(operation) => {
                for (name, scalar) in operation.form() {
                    let _ = visitor(ScalarNode::Binding(name));
                    scalar.visit(visitor);
                }
            }
            Self::Map(map) => map.iter().for_each(|(name, scalar)| {
                let _ = visitor(ScalarNode::Binding(name));
                scalar.visit(visitor);
            }),
            Self::Tuple(tuple) => tuple.iter().for_each(|scalar| scalar.visit(visitor)),
            Self::Application(definition) => definition.definition().visit(visitor),
        }
    }
}

/// A reference to a named value in a scope (e.g. "$self").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdRef(Id);

impl IdRef {
    pub fn new(id: Id) -> Self {
        Self(id)
    }

    pub fn id(&self) -> &Id {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for IdRef {
    type Err = hr_id::ParseError;

    fn from_str(id_ref: &str) -> Result<Self, Self::Err> {
        if !id_ref.starts_with('$') || id_ref.len() < 2 {
            Err(hr_id::ParseError::from(id_ref))
        } else {
            id_ref[1..].parse().map(Self::new)
        }
    }
}

impl From<Id> for IdRef {
    fn from(id: Id) -> Self {
        Self::new(id)
    }
}

impl From<IdRef> for Id {
    fn from(id_ref: IdRef) -> Self {
        id_ref.0
    }
}

/// The subject of an op.
///
/// Copied from the v1 `OpRef` model: an op may target either a concrete [`Link`] or a scoped
/// reference plus a suffix path.
///
/// ## v1-compatible JSON semantics
///
/// Encoded as a string:
///
/// - A concrete [`Link`] encodes as its string form (e.g. `"/lib/acme/foo/1.0.0"`).
/// - A scoped ref encodes as `"$id"` or `"$id/suffix/path"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Link(Link),
    Ref(IdRef, PathBuf),
}

pub const SCALAR_REF_PREFIX: PathLabel = path_label(&["state", "scalar", "ref"]);
pub const OPREF_PREFIX: PathLabel = path_label(&["state", "scalar", "ref", "op"]);
pub const OPDEF_PREFIX: PathLabel = path_label(&["state", "scalar", "op"]);
pub const OPDEF_REFLECT_PREFIX: PathLabel = path_label(&["state", "scalar", "op", "reflect"]);
pub const SCALAR_REFLECT_PREFIX: PathLabel = path_label(&["state", "scalar", "reflect"]);
pub const SCALAR_MAP: PathLabel = path_label(&["state", "scalar", "map"]);
pub const SCALAR_TUPLE: PathLabel = path_label(&["state", "scalar", "tuple"]);
pub const OPREF_GET: PathLabel = path_label(&["state", "scalar", "ref", "op", "get"]);
pub const OPREF_PUT: PathLabel = path_label(&["state", "scalar", "ref", "op", "put"]);
pub const OPREF_POST: PathLabel = path_label(&["state", "scalar", "ref", "op", "post"]);
pub const OPREF_DELETE: PathLabel = path_label(&["state", "scalar", "ref", "op", "delete"]);
pub const TCREF_COND: PathLabel = path_label(&["state", "scalar", "ref", "cond"]);
pub const TCREF_AFTER: PathLabel = path_label(&["state", "scalar", "ref", "after"]);
pub const TCREF_WHILE: PathLabel = path_label(&["state", "scalar", "ref", "while"]);
pub const TCREF_FOR_EACH: PathLabel = path_label(&["state", "scalar", "ref", "for_each"]);
pub const OPDEF_GET: PathLabel = path_label(&["state", "scalar", "op", "get"]);
pub const OPDEF_PUT: PathLabel = path_label(&["state", "scalar", "op", "put"]);
pub const OPDEF_POST: PathLabel = path_label(&["state", "scalar", "op", "post"]);
pub const OPDEF_DELETE: PathLabel = path_label(&["state", "scalar", "op", "delete"]);
pub const SCALAR_REFLECT_CLASS: PathLabel = path_label(&["state", "scalar", "reflect", "class"]);
pub const SCALAR_REFLECT_REF_PARTS: PathLabel =
    path_label(&["state", "scalar", "reflect", "ref_parts"]);
pub const OPDEF_REFLECT_FORM: PathLabel = path_label(&["state", "scalar", "op", "reflect", "form"]);
pub const OPDEF_REFLECT_LAST_ID: PathLabel =
    path_label(&["state", "scalar", "op", "reflect", "last_id"]);
pub const OPDEF_REFLECT_SCALARS: PathLabel =
    path_label(&["state", "scalar", "op", "reflect", "scalars"]);

impl de::FromStream for IdRef {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let id = String::from_stream((), decoder).await?;
        id.parse().map_err(de::Error::custom)
    }
}

impl<'en> en::IntoStream<'en> for IdRef {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(&self.to_string())
    }
}

impl<'en> en::ToStream<'en> for IdRef {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(&self.to_string())
    }
}

impl fmt::Display for IdRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.0)
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subject::Link(link) => fmt::Display::fmt(link, f),
            Subject::Ref(id, path) if path.is_empty() => fmt::Display::fmt(id, f),
            Subject::Ref(id, path) => write!(f, "{id}{path}"),
        }
    }
}

impl de::FromStream for Subject {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let s = String::from_stream((), decoder).await?;

        subject_from_str(&s).map_err(|err| de::Error::custom(err.to_string()))
    }
}

impl<'en> en::IntoStream<'en> for Subject {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        self.to_string().into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for Subject {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        en::IntoStream::into_stream(self.to_string(), encoder)
    }
}

impl de::FromStream for Scalar {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct ScalarVisitor;

        impl de::Visitor for ScalarVisitor {
            type Value = Scalar;

            fn expecting() -> &'static str {
                "a Scalar"
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::None))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::None))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::Number(Number::from(value))))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::Number(Number::from(value))))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::Number(Number::from(value))))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::Number(Number::from(value))))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(Scalar::Value(Value::String(value)))
            }

            async fn visit_seq<A: de::SeqAccess>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut items: Vec<Scalar> = if let Some(size) = seq.size_hint() {
                    Vec::with_capacity(size)
                } else {
                    Vec::new()
                };

                while let Some(value) = seq.next_element::<Scalar>(()).await? {
                    items.push(value);
                }

                Ok(Scalar::Tuple(items))
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let Some(key) = map.next_key::<String>(()).await? else {
                    return Ok(Scalar::Map(Map::new()));
                };

                if key.starts_with('/') {
                    if let Ok(identity) = key.parse::<crate::ApplicationIdentity>() {
                        if identity.kind() == crate::ApplicationKind::Class {
                            let definition = map.next_value::<Scalar>(()).await?;
                            if map.next_key::<de::IgnoredAny>(()).await?.is_some() {
                                return Err(de::Error::custom(
                                    "an application definition must contain exactly one URI entry",
                                ));
                            }
                            return Ok(Scalar::Application(Box::new(
                                crate::ApplicationDefinition::new(identity, definition),
                            )));
                        }
                    }
                    if let Some(value) = decode_typed_value_map_entry(&key, &mut map).await? {
                        return Ok(Scalar::Value(value));
                    }

                    let key_path = PathBuf::from_str(&key).ok();

                    if let Some(path) = key_path.as_ref() {
                        if let Some(op_def_type) = crate::op::OpDefType::from_path(path) {
                            let op_def =
                                crate::op::decode_opdef_map_entry(op_def_type, &mut map).await?;
                            return Ok(Scalar::Op(op_def));
                        }

                        if is_tcref_or_opref_path(path) {
                            let r = crate::tcref::decode_tcref_map_entry(key, &mut map).await?;
                            return Ok(Scalar::Ref(Box::new(r)));
                        }
                    }

                    let args = map.next_value::<crate::op::OpArgs>(()).await?;
                    if let crate::op::OpArgs::Seq(items) = &args {
                        if items.is_empty() {
                            if let Ok(link) = Link::from_str(&key) {
                                while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
                                    let _ = map.next_value::<de::IgnoredAny>(()).await?;
                                }
                                return Ok(Scalar::Value(Value::Link(link)));
                            }
                        }
                    }

                    let subject =
                        subject_from_str(&key).map_err(|err| de::Error::custom(err.to_string()))?;
                    let op = crate::op::opref_from_subject_args::<A::Error>(subject, args)?;
                    while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
                        let _ = map.next_value::<de::IgnoredAny>(()).await?;
                    }
                    return Ok(Scalar::Ref(Box::new(crate::tcref::TCRef::Op(op))));
                }

                if key.starts_with('$') {
                    let r = crate::tcref::decode_tcref_map_entry(key, &mut map).await?;
                    return Ok(Scalar::Ref(Box::new(r)));
                }

                let mut out = Map::new();
                let value = map.next_value::<Scalar>(()).await?;
                let id: Id = key
                    .parse::<Id>()
                    .map_err(|err| de::Error::custom(err.to_string()))?;
                out.insert(id, value);

                while let Some(key) = map.next_key::<String>(()).await? {
                    let value = map.next_value::<Scalar>(()).await?;
                    let id: Id = key
                        .parse::<Id>()
                        .map_err(|err| de::Error::custom(err.to_string()))?;
                    out.insert(id, value);
                }

                Ok(Scalar::Map(out))
            }
        }

        decoder.decode_any(ScalarVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for Scalar {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Scalar::Value(value) => value.into_stream(encoder),
            Scalar::Ref(r) => (*r).into_stream(encoder),
            Scalar::Op(op) => op.into_stream(encoder),
            Scalar::Map(map) => map.into_stream(encoder),
            Scalar::Tuple(tuple) => tuple.into_stream(encoder),
            Scalar::Application(definition) => definition.into_stream(encoder),
        }
    }
}

impl<'en> en::ToStream<'en> for Scalar {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

fn is_tcref_or_opref_path(path: &PathBuf) -> bool {
    path == &PathBuf::from(TCREF_COND)
        || path == &PathBuf::from(TCREF_AFTER)
        || path == &PathBuf::from(TCREF_WHILE)
        || path == &PathBuf::from(TCREF_FOR_EACH)
        || path == &PathBuf::from(OPREF_GET)
        || path == &PathBuf::from(OPREF_PUT)
        || path == &PathBuf::from(OPREF_POST)
        || path == &PathBuf::from(OPREF_DELETE)
}

pub(crate) fn subject_from_str(s: &str) -> Result<Subject, TCError> {
    if s.starts_with('$') {
        if let Some(i) = s.find('/') {
            let id = &s[..i];
            let path_str = &s[i..];
            let path =
                PathBuf::from_str(path_str).map_err(|err| TCError::bad_request(err.to_string()))?;
            let id_ref =
                IdRef::from_str(id).map_err(|err| TCError::bad_request(err.to_string()))?;
            Ok(Subject::Ref(id_ref, path))
        } else {
            let id_ref = IdRef::from_str(s).map_err(|err| TCError::bad_request(err.to_string()))?;
            Ok(Subject::Ref(id_ref, PathBuf::default()))
        }
    } else {
        Link::from_str(s).map(Subject::Link).map_err(TCError::from)
    }
}

impl Default for Scalar {
    fn default() -> Self {
        Scalar::Value(Value::default())
    }
}

impl From<Value> for Scalar {
    fn from(value: Value) -> Self {
        Scalar::Value(value)
    }
}

impl From<crate::tcref::TCRef> for Scalar {
    fn from(value: crate::tcref::TCRef) -> Self {
        Scalar::Ref(Box::new(value))
    }
}

impl From<crate::op::OpDef> for Scalar {
    fn from(value: crate::op::OpDef) -> Self {
        Scalar::Op(value)
    }
}

impl From<u64> for Scalar {
    fn from(value: u64) -> Self {
        Scalar::Value(Value::from(value))
    }
}

impl safecast::TryCastFrom<Scalar> for Value {
    fn can_cast_from(scalar: &Scalar) -> bool {
        match scalar {
            Scalar::Value(_) => true,
            Scalar::Tuple(items) => items.iter().all(Self::can_cast_from),
            Scalar::Map(_) | Scalar::Ref(_) | Scalar::Op(_) | Scalar::Application(_) => false,
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Self> {
        match scalar {
            Scalar::Value(value) => Some(value),
            Scalar::Tuple(items) => {
                let values = items
                    .into_iter()
                    .map(Self::opt_cast_from)
                    .collect::<Option<Vec<_>>>()?;
                Some(Value::Tuple(values))
            }
            Scalar::Map(_) | Scalar::Ref(_) | Scalar::Op(_) | Scalar::Application(_) => None,
        }
    }
}
