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
mod tests {
    use super::*;
    use crate::{IdRef, Scalar, TCRef};
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
