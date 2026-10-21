use std::collections::BTreeSet;
use std::sync::Arc;
use std::{fmt, str::FromStr};

use async_hash::{Digest, Hash, Output};
use destream::{de, en};
use pathlink::{path_label, Link, PathBuf, PathLabel};

use crate::{Id, Map, OpDef, OpRef, TCRef};
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
        let mut pending = vec![QueryNode::Scalar(self)];
        while let Some(node) = pending.pop() {
            match node {
                QueryNode::Children(mut children) => {
                    if let Some((_, value)) = children.next() {
                        pending.push(QueryNode::Children(children));
                        pending.push(QueryNode::Scalar(value));
                    }
                }
                QueryNode::Scalar(Scalar::Ref(_)) => return true,
                QueryNode::Scalar(Scalar::Map(map)) => {
                    pending.push(QueryNode::Children(Children::Map(map.iter())))
                }
                QueryNode::Scalar(Scalar::Tuple(tuple)) => {
                    pending.push(QueryNode::Children(Children::Tuple(tuple.iter())))
                }
                _ => {}
            }
        }
        false
    }

    /// Add the lexical bindings required to resolve this scalar.
    ///
    /// This is a syntactic query. It neither resolves nor mutates a runtime
    /// namespace; the caller owns the request-local accumulator.
    pub fn requires(&self, required: &mut BTreeSet<Id>) {
        collect_requires(QueryNode::Scalar(self), required)
    }

    /// Visit each method invoked on a concrete link in this scalar.
    ///
    /// This reports syntax only. The caller owns classification, aggregation,
    /// ordering, and policy.
    pub fn visit_referenced_methods(&self, visitor: &mut impl FnMut(&Link, crate::Method)) {
        visit_methods(self, visitor)
    }
}

// One iterative walk binds operation subjects, including nested arguments and
// lexical closures, without using the native stack for untrusted graph depth.
pub(crate) fn map_subjects(op: &mut crate::OpDef, visitor: &mut impl FnMut(&mut Subject)) {
    enum Node<'a> {
        Scalar(&'a mut Scalar),
        Op(&'a mut crate::OpDef),
        Ref(&'a mut crate::TCRef),
        Map(std::collections::btree_map::ValuesMut<'a, Id, Scalar>),
        Tuple(std::slice::IterMut<'a, Scalar>),
        Form(std::slice::IterMut<'a, (Id, Scalar)>),
    }
    let mut pending = vec![Node::Op(op)];
    while let Some(node) = pending.pop() {
        match node {
            Node::Map(mut items) => {
                if let Some(value) = items.next_back() {
                    pending.push(Node::Map(items));
                    pending.push(Node::Scalar(value));
                }
            }
            Node::Tuple(mut items) => {
                if let Some(value) = items.next_back() {
                    pending.push(Node::Tuple(items));
                    pending.push(Node::Scalar(value));
                }
            }
            Node::Form(mut items) => {
                if let Some((_, value)) = items.next_back() {
                    pending.push(Node::Form(items));
                    pending.push(Node::Scalar(value));
                }
            }
            Node::Op(op) => {
                let form = match op {
                    crate::OpDef::Get((_, form)) | crate::OpDef::Delete((_, form)) => form,
                    crate::OpDef::Put((_, _, form)) | crate::OpDef::Post(form) => form,
                };
                pending.push(Node::Form(form.iter_mut()));
            }
            Node::Scalar(Scalar::Value(_)) => {}
            Node::Scalar(Scalar::Op(op)) => pending.push(Node::Op(op)),
            Node::Scalar(Scalar::Map(map)) => pending.push(Node::Map(map.values_mut())),
            Node::Scalar(Scalar::Tuple(tuple)) => pending.push(Node::Tuple(tuple.iter_mut())),
            Node::Scalar(Scalar::Ref(reference)) => pending.push(Node::Ref(reference)),
            Node::Ref(reference) => match reference {
                crate::TCRef::Id(_) => {}
                crate::TCRef::Op(op) => {
                    let subject = match op {
                        crate::OpRef::Get((subject, key))
                        | crate::OpRef::Delete((subject, key)) => {
                            pending.push(Node::Scalar(key));
                            subject
                        }
                        crate::OpRef::Put((subject, key, value)) => {
                            pending.extend([Node::Scalar(key), Node::Scalar(value)]);
                            subject
                        }
                        crate::OpRef::Post((subject, params)) => {
                            pending.push(Node::Map(params.values_mut()));
                            subject
                        }
                    };
                    visitor(subject);
                }
                crate::TCRef::Cond(c) => pending.extend([
                    Node::Ref(&mut c.cond),
                    Node::Scalar(&mut c.then),
                    Node::Scalar(&mut c.or_else),
                ]),
                crate::TCRef::After(a) => {
                    pending.extend([Node::Scalar(&mut a.when), Node::Scalar(&mut a.then)])
                }
                crate::TCRef::While(w) => pending.extend([
                    Node::Scalar(&mut w.cond),
                    Node::Scalar(&mut w.closure),
                    Node::Scalar(&mut w.state),
                ]),
                crate::TCRef::ForEach(f) => {
                    pending.extend([Node::Scalar(&mut f.items), Node::Scalar(&mut f.op)])
                }
            },
        }
    }
}

pub(crate) fn validate_op(op: &crate::OpDef, visible: &BTreeSet<Id>) -> TCResult<()> {
    let mut pending = Vec::new();
    pending
        .try_reserve(1)
        .map_err(|err| TCError::internal(format!("cannot reserve validation frames: {err}")))?;
    pending.push((QueryNode::OpDef(op), Arc::new(visible.clone())));
    while let Some((node, visible)) = pending.pop() {
        pending
            .try_reserve(3)
            .map_err(|err| TCError::internal(format!("cannot reserve validation frames: {err}")))?;
        match node {
            QueryNode::Children(mut children) => {
                if let Some((_, value)) = children.next() {
                    pending.push((QueryNode::Children(children), Arc::clone(&visible)));
                    pending.push((QueryNode::Scalar(value), visible));
                }
            }
            QueryNode::Scalar(Scalar::Value(_)) => {}
            QueryNode::Scalar(Scalar::Ref(reference)) => {
                pending.push((QueryNode::Ref(reference), visible));
            }
            QueryNode::Scalar(Scalar::Op(op)) => {
                pending.push((QueryNode::OpDef(op), visible));
            }
            QueryNode::Scalar(Scalar::Map(map)) => {
                pending.push((QueryNode::Children(Children::Map(map.iter())), visible))
            }
            QueryNode::Scalar(Scalar::Tuple(tuple)) => {
                pending.push((QueryNode::Children(Children::Tuple(tuple.iter())), visible))
            }
            QueryNode::OpDef(op) => {
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
                pending.push((QueryNode::Children(Children::Form(op.form().iter())), scope));
            }
            QueryNode::Ref(crate::TCRef::Id(_)) => {}
            QueryNode::Ref(crate::TCRef::Op(op)) => {
                pending.push((QueryNode::OpRef(op), visible));
            }
            QueryNode::Ref(crate::TCRef::Cond(cond)) => {
                pending.push((QueryNode::Scalar(&cond.or_else), Arc::clone(&visible)));
                pending.push((QueryNode::Scalar(&cond.then), Arc::clone(&visible)));
                pending.push((QueryNode::Ref(&cond.cond), visible));
            }
            QueryNode::Ref(crate::TCRef::After(after)) => {
                pending.push((QueryNode::Scalar(&after.then), Arc::clone(&visible)));
                pending.push((QueryNode::Scalar(&after.when), visible));
            }
            QueryNode::Ref(crate::TCRef::While(while_ref)) => {
                pending.push((QueryNode::Scalar(&while_ref.state), Arc::clone(&visible)));
                let mut callback = (*visible).clone();
                crate::op::bind(
                    &mut callback,
                    &mut BTreeSet::new(),
                    &"state".parse().expect("reserved callback Id"),
                    "While callback parameter",
                )?;
                let callback = Arc::new(callback);
                pending.push((QueryNode::Scalar(&while_ref.closure), Arc::clone(&callback)));
                pending.push((QueryNode::Scalar(&while_ref.cond), callback));
            }
            QueryNode::Ref(crate::TCRef::ForEach(for_each)) => {
                pending.push((QueryNode::Scalar(&for_each.items), Arc::clone(&visible)));
                let mut body = (*visible).clone();
                crate::op::bind(
                    &mut body,
                    &mut BTreeSet::new(),
                    &for_each.item_name,
                    "ForEach item",
                )?;
                pending.push((QueryNode::Scalar(&for_each.op), Arc::new(body)));
            }
            QueryNode::OpRef(op) => match op {
                crate::OpRef::Get((_, key)) | crate::OpRef::Delete((_, key)) => {
                    pending.push((QueryNode::Scalar(key), visible))
                }
                crate::OpRef::Put((_, key, value)) => {
                    pending.push((QueryNode::Scalar(value), Arc::clone(&visible)));
                    pending.push((QueryNode::Scalar(key), visible));
                }
                crate::OpRef::Post((_, params)) => {
                    pending.push((QueryNode::Children(Children::Map(params.iter())), visible))
                }
            },
        }
    }
    Ok(())
}

enum Children<'a> {
    Tuple(std::slice::Iter<'a, Scalar>),
    Map(std::collections::btree_map::Iter<'a, Id, Scalar>),
    Form(std::slice::Iter<'a, (Id, Scalar)>),
}

impl<'a> Iterator for Children<'a> {
    type Item = (Option<&'a Id>, &'a Scalar);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Tuple(items) => items.next().map(|value| (None, value)),
            Self::Map(items) => items.next().map(|(id, value)| (Some(id), value)),
            Self::Form(items) => items.next().map(|(id, value)| (Some(id), value)),
        }
    }
}

enum QueryNode<'a> {
    Children(Children<'a>),
    Scalar(&'a Scalar),
    Ref(&'a crate::TCRef),
    OpRef(&'a crate::OpRef),
    OpDef(&'a crate::OpDef),
}

pub(crate) fn collect_op_requires(op: &crate::OpDef, required: &mut BTreeSet<Id>) {
    collect_requires(QueryNode::OpDef(op), required)
}

pub(crate) fn collect_op_ref_requires(op: &crate::OpRef, required: &mut BTreeSet<Id>) {
    collect_requires(QueryNode::OpRef(op), required)
}

pub(crate) fn collect_ref_requires(reference: &crate::TCRef, required: &mut BTreeSet<Id>) {
    collect_requires(QueryNode::Ref(reference), required)
}

fn collect_requires(initial: QueryNode<'_>, required: &mut BTreeSet<Id>) {
    let mut pending = vec![(initial, Arc::new(BTreeSet::new()))];
    while let Some((node, bound)) = pending.pop() {
        match node {
            QueryNode::Children(mut children) => {
                if let Some((_, value)) = children.next() {
                    pending.push((QueryNode::Children(children), Arc::clone(&bound)));
                    pending.push((QueryNode::Scalar(value), bound));
                }
            }
            QueryNode::Scalar(Scalar::Value(_)) => {}
            QueryNode::Scalar(Scalar::Ref(reference)) => {
                pending.push((QueryNode::Ref(reference), bound));
            }
            QueryNode::Scalar(Scalar::Op(op)) => {
                pending.push((QueryNode::OpDef(op), bound));
            }
            QueryNode::Scalar(Scalar::Map(map)) => {
                pending.push((QueryNode::Children(Children::Map(map.iter())), bound))
            }
            QueryNode::Scalar(Scalar::Tuple(tuple)) => {
                pending.push((QueryNode::Children(Children::Tuple(tuple.iter())), bound))
            }
            QueryNode::OpDef(op) => {
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
                pending.push((QueryNode::Children(Children::Form(op.form().iter())), local));
            }
            QueryNode::Ref(crate::TCRef::Id(id)) => {
                if id.as_str() != "self" && !bound.contains(id.id()) {
                    required.insert(id.id().clone());
                }
            }
            QueryNode::Ref(crate::TCRef::Op(op)) => {
                pending.push((QueryNode::OpRef(op), bound));
            }
            QueryNode::Ref(crate::TCRef::Cond(cond)) => {
                pending.push((QueryNode::Scalar(&cond.or_else), Arc::clone(&bound)));
                pending.push((QueryNode::Scalar(&cond.then), Arc::clone(&bound)));
                pending.push((QueryNode::Ref(&cond.cond), bound));
            }
            QueryNode::Ref(crate::TCRef::After(after)) => {
                pending.push((QueryNode::Scalar(&after.then), Arc::clone(&bound)));
                pending.push((QueryNode::Scalar(&after.when), bound));
            }
            QueryNode::Ref(crate::TCRef::While(while_ref)) => {
                pending.push((QueryNode::Scalar(&while_ref.state), Arc::clone(&bound)));
                let mut callback = (*bound).clone();
                callback.insert("state".parse().expect("reserved callback Id"));
                let callback = Arc::new(callback);
                pending.push((QueryNode::Scalar(&while_ref.closure), Arc::clone(&callback)));
                pending.push((QueryNode::Scalar(&while_ref.cond), callback));
            }
            QueryNode::Ref(crate::TCRef::ForEach(for_each)) => {
                pending.push((QueryNode::Scalar(&for_each.items), Arc::clone(&bound)));
                let mut body = (*bound).clone();
                body.insert(for_each.item_name.clone());
                pending.push((QueryNode::Scalar(&for_each.op), Arc::new(body)));
            }
            QueryNode::OpRef(op) => {
                let subject = match op {
                    crate::OpRef::Get((subject, key)) | crate::OpRef::Delete((subject, key)) => {
                        pending.push((QueryNode::Scalar(key), Arc::clone(&bound)));
                        subject
                    }
                    crate::OpRef::Put((subject, key, value)) => {
                        pending.push((QueryNode::Scalar(value), Arc::clone(&bound)));
                        pending.push((QueryNode::Scalar(key), Arc::clone(&bound)));
                        subject
                    }
                    crate::OpRef::Post((subject, params)) => {
                        pending.push((
                            QueryNode::Children(Children::Map(params.iter())),
                            Arc::clone(&bound),
                        ));
                        subject
                    }
                };
                if let crate::Subject::Ref(id, _) = subject {
                    if id.as_str() != "self" && !bound.contains(id.id()) {
                        required.insert(id.id().clone());
                    }
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

#[derive(Clone, Copy)]
enum MethodNode<'a> {
    Scalar(&'a Scalar),
    Ref(&'a TCRef),
    Op(&'a OpDef),
    Call(&'a OpRef),
}

enum MethodChildren<'a> {
    Fixed(std::array::IntoIter<Option<MethodNode<'a>>, 3>),
    Tuple(std::slice::Iter<'a, Scalar>),
    Map(std::collections::btree_map::Iter<'a, Id, Scalar>),
    Form(std::slice::Iter<'a, (Id, Scalar)>),
}

impl<'a> Iterator for MethodChildren<'a> {
    type Item = (Option<&'a Id>, MethodNode<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Fixed(items) => items.flatten().next().map(|node| (None, node)),
            Self::Tuple(items) => items.next().map(|value| (None, MethodNode::Scalar(value))),
            Self::Map(items) => items
                .next()
                .map(|(key, value)| (Some(key), MethodNode::Scalar(value))),
            Self::Form(items) => items
                .next()
                .map(|(key, value)| (Some(key), MethodNode::Scalar(value))),
        }
    }
}

fn method_children(node: MethodNode<'_>) -> MethodChildren<'_> {
    use MethodNode::{Call as C, Op as O, Ref as R, Scalar as S};
    let fields = match node {
        S(value) => match &value {
            Scalar::Value(_) => [None, None, None],
            Scalar::Ref(value) => [Some(R(value)), None, None],
            Scalar::Op(value) => [Some(O(value)), None, None],
            Scalar::Map(values) => return MethodChildren::Map(values.iter()),
            Scalar::Tuple(values) => return MethodChildren::Tuple(values.iter()),
        },
        R(value) => match &value {
            TCRef::Id(_) => [None, None, None],
            TCRef::Op(value) => [Some(C(value)), None, None],
            TCRef::Cond(v) => [Some(R(&v.cond)), Some(S(&v.then)), Some(S(&v.or_else))],
            TCRef::After(v) => [Some(S(&v.when)), Some(S(&v.then)), None],
            TCRef::While(v) => [Some(S(&v.cond)), Some(S(&v.closure)), Some(S(&v.state))],
            TCRef::ForEach(v) => [Some(S(&v.items)), Some(S(&v.op)), None],
        },
        O(value) => return MethodChildren::Form(value.form().iter()),
        C(value) => match value {
            OpRef::Get((_, key)) | OpRef::Delete((_, key)) => [Some(S(key)), None, None],
            OpRef::Put((_, key, value)) => [Some(S(key)), Some(S(value)), None],
            OpRef::Post((_, params)) => return MethodChildren::Map(params.iter()),
        },
    };
    MethodChildren::Fixed(fields.into_iter())
}

fn visit_methods(root: &Scalar, visitor: &mut impl FnMut(&pathlink::Link, crate::Method)) {
    let mut frames = vec![(
        MethodNode::Scalar(root),
        method_children(MethodNode::Scalar(root)),
    )];
    while let Some((_, children)) = frames.last_mut() {
        if let Some((_, next)) = children.next() {
            frames.push((next, method_children(next)));
            continue;
        }
        let (node, _) = frames.pop().unwrap();
        if let MethodNode::Call(op) = node {
            let (subject, method) = match op {
                OpRef::Get((s, _)) => (s, crate::Method::Get),
                OpRef::Put((s, _, _)) => (s, crate::Method::Put),
                OpRef::Post((s, _)) => (s, crate::Method::Post),
                OpRef::Delete((s, _)) => (s, crate::Method::Delete),
            };
            if let Subject::Link(link) = subject {
                visitor(link, method);
            }
        }
    }
}
