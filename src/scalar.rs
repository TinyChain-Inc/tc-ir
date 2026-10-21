use std::collections::BTreeSet;
use std::{fmt, str::FromStr};

use async_hash::{Digest, Hash, Output};
use destream::{de, en};
use pathlink::{path_label, Link, PathBuf, PathLabel};

use crate::{Id, Map};
use tc_error::{TCError, TCResult};
use tc_value::Value;

/// Scalar values exchanged via the TinyChain IR.
/// Native construction must respect the codec's structural nesting contract.
#[derive(Clone, PartialEq)]
pub enum Scalar {
    Value(Value),
    Ref(Box<crate::TCRef>),
    Op(crate::OpDef),
    Map(Map<Scalar>),
    Tuple(Vec<Scalar>),
}

impl fmt::Debug for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(f, "Value({value:?})"),
            Self::Ref(value) => write!(f, "Ref({value:?})"),
            Self::Op(value) => write!(f, "Op({value:?})"),
            Self::Tuple(values) => write!(f, "Tuple({values:?})"),
            Self::Map(values) => write!(f, "Map({:?})", **values),
        }
    }
}

impl get_size::GetSize for Scalar {
    fn get_heap_size(&self) -> usize {
        match self {
            Self::Value(value) => value.get_heap_size(),
            Self::Ref(value) => value.get_heap_size(),
            Self::Op(value) => value.get_heap_size(),
            Self::Map(values) => values.get_heap_size(),
            Self::Tuple(values) => values.get_heap_size(),
        }
    }
}

impl Scalar {
    /// Whether this value contains an expression requiring evaluation.
    /// Operation definitions are literal closures, not evaluated expressions.
    pub fn is_ref(&self) -> bool {
        match self {
            Self::Ref(_) => true,
            Self::Map(values) => values.values().any(Self::is_ref),
            Self::Tuple(values) => values.iter().any(Self::is_ref),
            Self::Value(_) | Self::Op(_) => false,
        }
    }

    /// Add the lexical bindings required to resolve this scalar.
    ///
    /// This is a syntactic query. It neither resolves nor mutates a runtime
    /// namespace; the caller owns the request-local accumulator.
    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        self.requires_in(&BTreeSet::new(), required)
    }

    /// Visit each method invoked on a concrete link in this scalar.
    ///
    /// This reports syntax only. The caller owns classification, aggregation,
    /// ordering, and policy.
    pub fn visit_referenced_methods(&self, visitor: &mut impl FnMut(&Link, crate::Method)) {
        match self {
            Self::Value(_) => {}
            Self::Ref(value) => value.visit_referenced_methods(visitor),
            Self::Op(value) => value.visit_referenced_methods(visitor),
            Self::Map(values) => {
                for value in values.values() {
                    value.visit_referenced_methods(visitor);
                }
            }
            Self::Tuple(values) => {
                for value in values {
                    value.visit_referenced_methods(visitor);
                }
            }
        }
    }

    pub(crate) fn requires_in(&self, bound: &BTreeSet<Id>, required: &mut BTreeSet<Id>) {
        match self {
            Self::Value(_) => {}
            Self::Ref(value) => value.requires_in(bound, required),
            Self::Op(value) => value.requires_in(bound, required),
            Self::Map(values) => {
                for value in values.values() {
                    value.requires_in(bound, required);
                }
            }
            Self::Tuple(values) => {
                for value in values {
                    value.requires_in(bound, required);
                }
            }
        }
    }

    pub(crate) fn validate_in(&self, visible: &BTreeSet<Id>) -> TCResult<()> {
        match self {
            Self::Value(_) => {}
            Self::Ref(value) => value.validate_in(visible)?,
            Self::Op(value) => value.validate_in(visible)?,
            Self::Map(values) => {
                for value in values.values() {
                    value.validate_in(visible)?;
                }
            }
            Self::Tuple(values) => {
                for value in values {
                    value.validate_in(visible)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn map_subjects(&mut self, visitor: &mut impl FnMut(&mut Subject)) {
        match self {
            Self::Value(_) => {}
            Self::Ref(value) => value.map_subjects(visitor),
            Self::Op(value) => value.map_subjects(visitor),
            Self::Map(values) => {
                for value in values.values_mut().rev() {
                    value.map_subjects(visitor);
                }
            }
            Self::Tuple(values) => {
                for value in values.iter_mut().rev() {
                    value.map_subjects(visitor);
                }
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
            Scalar::Ref(value) => Hash::<D>::hash(value.as_ref()),
            Scalar::Op(value) => Hash::<D>::hash(value),
            Scalar::Map(values) => Hash::<D>::hash(values),
            Scalar::Tuple(values) => Hash::<D>::hash(values),
        }
    }
}

/// A reference to a named value in a scope (e.g. "$self").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdRef(Id);

impl get_size::GetSize for IdRef {
    fn get_heap_size(&self) -> usize {
        self.0.get_heap_size()
    }
}

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
/// An op targets either a concrete [`Link`] or a scoped reference plus a suffix path.
///
/// ## Wire representation
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

impl get_size::GetSize for Subject {
    fn get_heap_size(&self) -> usize {
        match self {
            Self::Link(link) => link.get_heap_size(),
            Self::Ref(id, path) => id.get_heap_size() + path.get_heap_size(),
        }
    }
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
        Value::default().into()
    }
}

impl From<Value> for Scalar {
    fn from(value: Value) -> Self {
        Scalar::Value(value)
    }
}

impl From<crate::TCRef> for Scalar {
    fn from(value: crate::TCRef) -> Self {
        Scalar::Ref(Box::new(value))
    }
}

impl From<crate::OpDef> for Scalar {
    fn from(value: crate::OpDef) -> Self {
        Scalar::Op(value)
    }
}

impl From<u64> for Scalar {
    fn from(value: u64) -> Self {
        Value::from(value).into()
    }
}

impl safecast::TryCastFrom<Scalar> for Value {
    fn can_cast_from(scalar: &Scalar) -> bool {
        let mut pending = vec![std::slice::from_ref(scalar).iter()];
        while let Some(items) = pending.last_mut() {
            match items.next() {
                Some(Scalar::Value(value)) => {
                    if !value.within_depth(tc_value::MAX_VALUE_DEPTH - (pending.len() - 1)) {
                        return false;
                    }
                }
                Some(Scalar::Tuple(items)) => {
                    if pending.len() > tc_value::MAX_VALUE_DEPTH {
                        return false;
                    }
                    pending.push(items.iter());
                }
                Some(_) => return false,
                None => {
                    pending.pop();
                }
            }
        }
        true
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Self> {
        if !Self::can_cast_from(&scalar) {
            return None;
        }

        scalar_into_value(scalar)
    }
}

// The caller validates the complete shape, including embedded Values, once.
fn scalar_into_value(scalar: Scalar) -> Option<Value> {
    match scalar {
        Scalar::Value(value) => Some(value),
        Scalar::Tuple(items) => items
            .into_iter()
            .map(scalar_into_value)
            .collect::<Option<Vec<_>>>()
            .map(Value::Tuple),
        _ => None,
    }
}
