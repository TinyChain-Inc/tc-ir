use std::collections::BTreeSet;
use std::sync::Arc;
use std::{fmt, str::FromStr};

use async_hash::{Digest, Hash, Output};
use destream::{de, en, IntoStream};
use number_general::Number;
use pathlink::{path_label, Link, PathBuf, PathLabel};
use tc_error::{TCError, TCResult};
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
}

impl Scalar {
    /// Add the lexical bindings required to resolve this scalar.
    ///
    /// This is a syntactic query. It neither resolves nor mutates a runtime
    /// namespace; the caller owns the request-local accumulator.
    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        collect_requires(RequireNode::Scalar(self), required)
    }

    /// Visit each method invoked on a concrete link in this scalar.
    ///
    /// This reports syntax only. The caller owns classification, aggregation,
    /// ordering, and policy.
    pub fn visit_referenced_methods(&self, visitor: &mut impl FnMut(&Link, crate::Method)) {
        match self {
            Self::Value(_) => {}
            Self::Ref(reference) => reference.visit_referenced_methods(visitor),
            Self::Op(op) => {
                for (_, scalar) in op.form() {
                    scalar.visit_referenced_methods(visitor);
                }
            }
            Self::Map(map) => {
                for scalar in map.values() {
                    scalar.visit_referenced_methods(visitor);
                }
            }
            Self::Tuple(tuple) => {
                for scalar in tuple {
                    scalar.visit_referenced_methods(visitor);
                }
            }
        }
    }
}

enum ValidateNode<'a> {
    Scalar(&'a Scalar),
    Ref(&'a crate::TCRef),
    OpRef(&'a crate::OpRef),
    OpDef(&'a crate::OpDef),
}

pub(crate) fn validate_op(op: &crate::OpDef, visible: &BTreeSet<Id>) -> TCResult<()> {
    let mut pending = vec![(ValidateNode::OpDef(op), Arc::new(visible.clone()))];
    while let Some((node, visible)) = pending.pop() {
        match node {
            ValidateNode::Scalar(Scalar::Value(_)) => {}
            ValidateNode::Scalar(Scalar::Ref(reference)) => {
                pending.push((ValidateNode::Ref(reference), visible));
            }
            ValidateNode::Scalar(Scalar::Op(op)) => {
                pending.push((ValidateNode::OpDef(op), visible));
            }
            ValidateNode::Scalar(Scalar::Map(map)) => pending.extend(
                map.values()
                    .rev()
                    .map(|scalar| (ValidateNode::Scalar(scalar), Arc::clone(&visible))),
            ),
            ValidateNode::Scalar(Scalar::Tuple(tuple)) => pending.extend(
                tuple
                    .iter()
                    .rev()
                    .map(|scalar| (ValidateNode::Scalar(scalar), Arc::clone(&visible))),
            ),
            ValidateNode::OpDef(op) => {
                if op.form().is_empty() {
                    return Err(TCError::bad_request(
                        "an OpDef must define at least one provider",
                    ));
                }
                let mut scope = (*visible).clone();
                let mut local = BTreeSet::new();
                match op {
                    crate::OpDef::Get((key, _)) | crate::OpDef::Delete((key, _)) => {
                        crate::op::bind(&mut scope, &mut local, key, "parameter")?;
                    }
                    crate::OpDef::Put((key, value, _)) => {
                        crate::op::bind(&mut scope, &mut local, key, "parameter")?;
                        crate::op::bind(&mut scope, &mut local, value, "parameter")?;
                    }
                    crate::OpDef::Post(_) => {}
                }
                for (id, _) in op.form() {
                    crate::op::bind(&mut scope, &mut local, id, "provider")?;
                }
                let scope = Arc::new(scope);
                pending.extend(
                    op.form()
                        .iter()
                        .rev()
                        .map(|(_, scalar)| (ValidateNode::Scalar(scalar), Arc::clone(&scope))),
                );
            }
            ValidateNode::Ref(crate::TCRef::Id(_)) => {}
            ValidateNode::Ref(crate::TCRef::Op(op)) => {
                pending.push((ValidateNode::OpRef(op), visible));
            }
            ValidateNode::Ref(crate::TCRef::Cond(cond)) => {
                pending.push((ValidateNode::Scalar(&cond.or_else), Arc::clone(&visible)));
                pending.push((ValidateNode::Scalar(&cond.then), Arc::clone(&visible)));
                pending.push((ValidateNode::Ref(&cond.cond), visible));
            }
            ValidateNode::Ref(crate::TCRef::After(after)) => {
                pending.push((ValidateNode::Scalar(&after.then), Arc::clone(&visible)));
                pending.push((ValidateNode::Scalar(&after.when), visible));
            }
            ValidateNode::Ref(crate::TCRef::While(while_ref)) => {
                pending.push((ValidateNode::Scalar(&while_ref.state), Arc::clone(&visible)));
                let mut callback = (*visible).clone();
                crate::op::bind(
                    &mut callback,
                    &mut BTreeSet::new(),
                    &"state".parse().expect("reserved callback Id"),
                    "While callback parameter",
                )?;
                let callback = Arc::new(callback);
                pending.push((
                    ValidateNode::Scalar(&while_ref.closure),
                    Arc::clone(&callback),
                ));
                pending.push((ValidateNode::Scalar(&while_ref.cond), callback));
            }
            ValidateNode::Ref(crate::TCRef::ForEach(for_each)) => {
                pending.push((ValidateNode::Scalar(&for_each.items), Arc::clone(&visible)));
                let mut body = (*visible).clone();
                crate::op::bind(
                    &mut body,
                    &mut BTreeSet::new(),
                    &for_each.item_name,
                    "ForEach item",
                )?;
                pending.push((ValidateNode::Scalar(&for_each.op), Arc::new(body)));
            }
            ValidateNode::OpRef(op) => {
                let scalars: Vec<&Scalar> = match op {
                    crate::OpRef::Get((_, key)) | crate::OpRef::Delete((_, key)) => vec![key],
                    crate::OpRef::Put((_, key, value)) => vec![key, value],
                    crate::OpRef::Post((_, params)) => params.values().collect(),
                };
                pending.extend(
                    scalars
                        .into_iter()
                        .rev()
                        .map(|scalar| (ValidateNode::Scalar(scalar), Arc::clone(&visible))),
                );
            }
        }
    }
    Ok(())
}

enum RequireNode<'a> {
    Scalar(&'a Scalar),
    Ref(&'a crate::TCRef),
    OpRef(&'a crate::OpRef),
    OpDef(&'a crate::OpDef),
}

pub(crate) fn collect_op_requires(op: &crate::OpDef, required: &mut BTreeSet<Id>) {
    collect_requires(RequireNode::OpDef(op), required)
}

pub(crate) fn collect_op_ref_requires(op: &crate::OpRef, required: &mut BTreeSet<Id>) {
    collect_requires(RequireNode::OpRef(op), required)
}

pub(crate) fn collect_ref_requires(reference: &crate::TCRef, required: &mut BTreeSet<Id>) {
    collect_requires(RequireNode::Ref(reference), required)
}

fn collect_requires(initial: RequireNode<'_>, required: &mut BTreeSet<Id>) {
    let mut pending = vec![(initial, Arc::new(BTreeSet::new()))];
    while let Some((node, bound)) = pending.pop() {
        match node {
            RequireNode::Scalar(Scalar::Value(_)) => {}
            RequireNode::Scalar(Scalar::Ref(reference)) => {
                pending.push((RequireNode::Ref(reference), bound));
            }
            RequireNode::Scalar(Scalar::Op(op)) => {
                pending.push((RequireNode::OpDef(op), bound));
            }
            RequireNode::Scalar(Scalar::Map(map)) => {
                pending.extend(
                    map.values()
                        .rev()
                        .map(|scalar| (RequireNode::Scalar(scalar), Arc::clone(&bound))),
                );
            }
            RequireNode::Scalar(Scalar::Tuple(tuple)) => {
                pending.extend(
                    tuple
                        .iter()
                        .rev()
                        .map(|scalar| (RequireNode::Scalar(scalar), Arc::clone(&bound))),
                );
            }
            RequireNode::OpDef(op) => {
                let mut local = (*bound).clone();
                match op {
                    crate::OpDef::Get((key, _)) | crate::OpDef::Delete((key, _)) => {
                        local.insert(key.clone());
                    }
                    crate::OpDef::Put((key, value, _)) => {
                        local.insert(key.clone());
                        local.insert(value.clone());
                    }
                    crate::OpDef::Post(_) => {}
                }
                local.extend(op.form().iter().map(|(id, _)| id.clone()));
                let local = Arc::new(local);
                pending.extend(
                    op.form()
                        .iter()
                        .rev()
                        .map(|(_, scalar)| (RequireNode::Scalar(scalar), Arc::clone(&local))),
                );
            }
            RequireNode::Ref(crate::TCRef::Id(id)) => {
                if id.as_str() != "self" && !bound.contains(id.id()) {
                    required.insert(id.id().clone());
                }
            }
            RequireNode::Ref(crate::TCRef::Op(op)) => {
                pending.push((RequireNode::OpRef(op), bound));
            }
            RequireNode::Ref(crate::TCRef::Cond(cond)) => {
                pending.push((RequireNode::Scalar(&cond.or_else), Arc::clone(&bound)));
                pending.push((RequireNode::Scalar(&cond.then), Arc::clone(&bound)));
                pending.push((RequireNode::Ref(&cond.cond), bound));
            }
            RequireNode::Ref(crate::TCRef::After(after)) => {
                pending.push((RequireNode::Scalar(&after.then), Arc::clone(&bound)));
                pending.push((RequireNode::Scalar(&after.when), bound));
            }
            RequireNode::Ref(crate::TCRef::While(while_ref)) => {
                pending.push((RequireNode::Scalar(&while_ref.state), Arc::clone(&bound)));
                let mut callback = (*bound).clone();
                callback.insert("state".parse().expect("reserved callback Id"));
                let callback = Arc::new(callback);
                pending.push((
                    RequireNode::Scalar(&while_ref.closure),
                    Arc::clone(&callback),
                ));
                pending.push((RequireNode::Scalar(&while_ref.cond), callback));
            }
            RequireNode::Ref(crate::TCRef::ForEach(for_each)) => {
                pending.push((RequireNode::Scalar(&for_each.items), Arc::clone(&bound)));
                let mut body = (*bound).clone();
                body.insert(for_each.item_name.clone());
                pending.push((RequireNode::Scalar(&for_each.op), Arc::new(body)));
            }
            RequireNode::OpRef(op) => {
                let (subject, scalars): (&crate::Subject, Vec<&Scalar>) = match op {
                    crate::OpRef::Get((subject, key)) | crate::OpRef::Delete((subject, key)) => {
                        (subject, vec![key])
                    }
                    crate::OpRef::Put((subject, key, value)) => (subject, vec![key, value]),
                    crate::OpRef::Post((subject, params)) => (subject, params.values().collect()),
                };
                if let crate::Subject::Ref(id, _) = subject {
                    if id.as_str() != "self" && !bound.contains(id.id()) {
                        required.insert(id.id().clone());
                    }
                }
                pending.extend(
                    scalars
                        .into_iter()
                        .rev()
                        .map(|scalar| (RequireNode::Scalar(scalar), Arc::clone(&bound))),
                );
            }
        }
    }
}

impl<D: Digest> Hash<D> for Scalar {
    fn hash(self) -> Output<D> {
        Hash::<D>::hash(&self)
    }
}

impl<D: Digest> Hash<D> for &Scalar {
    fn hash(self) -> Output<D> {
        match self {
            Scalar::Value(value) => Hash::<D>::hash(value),
            Scalar::Ref(reference) => Hash::<D>::hash(reference.as_ref()),
            Scalar::Op(op) => Hash::<D>::hash(op),
            Scalar::Map(map) => Hash::<D>::hash(map),
            Scalar::Tuple(tuple) => Hash::<D>::hash(tuple),
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

impl<D: Digest> Hash<D> for &IdRef {
    fn hash(self) -> Output<D> {
        Hash::<D>::hash(&self.0)
    }
}

impl<D: Digest> Hash<D> for &Subject {
    fn hash(self) -> Output<D> {
        Hash::<D>::hash(self.to_string())
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
                    if out.insert(id.clone(), value).is_some() {
                        return Err(de::Error::custom(format!("duplicate map key {id}")));
                    }
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
            Scalar::Map(_) | Scalar::Ref(_) | Scalar::Op(_) => false,
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
            Scalar::Map(_) | Scalar::Ref(_) | Scalar::Op(_) => None,
        }
    }
}
