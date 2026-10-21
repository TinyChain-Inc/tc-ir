use std::collections::BTreeSet;

use async_hash::{Digest, Hash, Output};
use pathlink::PathBuf;

use crate::{Id, Map, Scalar, Subject};
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
/// Resolution and execution belong to the host kernel.
///
/// ## Wire representation
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

    pub(crate) fn path(&self) -> PathBuf {
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

impl get_size::GetSize for OpRef {
    fn get_heap_size(&self) -> usize {
        match self {
            Self::Get((subject, key)) | Self::Delete((subject, key)) => {
                subject.get_heap_size() + key.get_heap_size()
            }
            Self::Put((subject, key, value)) => {
                subject.get_heap_size() + key.get_heap_size() + value.get_heap_size()
            }
            Self::Post((subject, params)) => subject.get_heap_size() + params.get_heap_size(),
        }
    }
}

impl get_size::GetSize for OpDef {
    fn get_heap_size(&self) -> usize {
        match self {
            Self::Get((key, form)) | Self::Delete((key, form)) => {
                key.get_heap_size() + form.get_heap_size()
            }
            Self::Put((key, value, form)) => {
                key.get_heap_size() + value.get_heap_size() + form.get_heap_size()
            }
            Self::Post(form) => form.get_heap_size(),
        }
    }
}

impl OpDef {
    /// Bind concrete references beneath this resource to its native `$self`.
    /// Replicated methods use this so their local writes are not forwarded again.
    pub fn reference_self(mut self, resource: &pathlink::Link) -> Self {
        crate::scalar::map_subjects(&mut self, &mut |subject| {
            if let Subject::Link(link) = subject {
                if link.host() == resource.host() && link.path().starts_with(resource.path()) {
                    *subject = Subject::Ref(
                        crate::IdRef::new("self".parse().expect("self identifier")),
                        PathBuf::from_slice(&link.path()[resource.path().len()..]),
                    );
                }
            }
        });
        self
    }

    /// Bind `$self` operations to the owning resource's public invocation boundary.
    /// Composed methods use this so nested writes participate in replication.
    pub fn dereference_self(mut self, resource: &pathlink::Link) -> Self {
        crate::scalar::map_subjects(&mut self, &mut |subject| {
            if let Subject::Ref(id, suffix) = subject {
                if id.as_str() == "self" {
                    let mut link = resource.clone();
                    for segment in suffix.iter() {
                        link = link.append(segment.clone());
                    }
                    *subject = Subject::Link(link);
                }
            }
        });
        self
    }

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

    pub(crate) fn class(&self) -> OpDefType {
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

#[cfg(test)]
#[path = "../tests/unit/op.rs"]
mod tests;
