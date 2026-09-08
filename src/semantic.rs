//! Format-neutral semantic hashing for immutable TinyChain definitions.

use sha2::{Digest as _, Sha256};
use tc_value::Value;

use crate::{
    ApplicationDefinition, ApplicationIdentity, Digest, Id, Map, OpDef, OpRef, Scalar, ScalarNode,
    Subject, TCRef,
};

/// A deterministic SHA-256 traversal independent of any wire codec.
pub struct SemanticHasher(Sha256);

impl SemanticHasher {
    pub fn new(domain: &str) -> Self {
        let mut hasher = Self(Sha256::new());
        hasher.write_bytes(domain.as_bytes());
        hasher
    }

    pub fn write_tag(&mut self, tag: &str) {
        self.write_bytes(tag.as_bytes());
    }

    pub fn write_len(&mut self, len: usize) {
        self.0.update((len as u64).to_be_bytes());
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_len(bytes.len());
        self.0.update(bytes);
    }

    pub fn finish(self) -> Digest {
        Digest::from_sha256(self.0.finalize().into())
    }
}

/// One canonical semantic digest traversal.
pub trait SemanticHash {
    fn write_semantic(&self, hasher: &mut SemanticHasher);

    fn semantic_digest(&self) -> Digest {
        let mut hasher = SemanticHasher::new("tinychain.semantic.v1");
        self.write_semantic(&mut hasher);
        hasher.finish()
    }
}

impl SemanticHash for Id {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        hasher.write_bytes(self.as_str().as_bytes());
    }
}

impl SemanticHash for ApplicationIdentity {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        hasher.write_tag("application_identity");
        hasher.write_bytes(self.to_string().as_bytes());
    }
}

impl SemanticHash for Value {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        match self {
            Self::None => hasher.write_tag("value.none"),
            Self::Bytes(bytes) => {
                hasher.write_tag("value.bytes");
                hasher.write_bytes(bytes);
            }
            Self::Link(link) => {
                hasher.write_tag("value.link");
                hasher.write_bytes(link.to_string().as_bytes());
            }
            Self::Number(number) => {
                hasher.write_tag("value.number");
                hasher.write_bytes(number.to_string().as_bytes());
            }
            Self::String(string) => {
                hasher.write_tag("value.string");
                hasher.write_bytes(string.as_bytes());
            }
            Self::Tuple(tuple) => {
                hasher.write_tag("value.tuple");
                hasher.write_len(tuple.len());
                for value in tuple {
                    value.write_semantic(hasher);
                }
            }
        }
    }
}

impl<T: SemanticHash> SemanticHash for Map<T> {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        hasher.write_tag("map");
        hasher.write_len(self.len());
        for (key, value) in self {
            key.write_semantic(hasher);
            value.write_semantic(hasher);
        }
    }
}

impl SemanticHash for Subject {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        hasher.write_tag("subject");
        hasher.write_bytes(self.to_string().as_bytes());
    }
}

impl SemanticHash for OpRef {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        match self {
            Self::Get((subject, key)) => {
                hasher.write_tag("op_ref.get");
                subject.write_semantic(hasher);
                key.write_semantic(hasher);
            }
            Self::Put((subject, key, value)) => {
                hasher.write_tag("op_ref.put");
                subject.write_semantic(hasher);
                key.write_semantic(hasher);
                value.write_semantic(hasher);
            }
            Self::Post((subject, params)) => {
                hasher.write_tag("op_ref.post");
                subject.write_semantic(hasher);
                params.write_semantic(hasher);
            }
            Self::Delete((subject, key)) => {
                hasher.write_tag("op_ref.delete");
                subject.write_semantic(hasher);
                key.write_semantic(hasher);
            }
        }
    }
}

fn write_form(form: &[(Id, Scalar)], hasher: &mut SemanticHasher) {
    hasher.write_len(form.len());
    for (name, value) in form {
        name.write_semantic(hasher);
        value.write_semantic(hasher);
    }
}

impl SemanticHash for OpDef {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        match self {
            Self::Get((key, form)) => {
                hasher.write_tag("op_def.get");
                key.write_semantic(hasher);
                write_form(form, hasher);
            }
            Self::Put((key, value, form)) => {
                hasher.write_tag("op_def.put");
                key.write_semantic(hasher);
                value.write_semantic(hasher);
                write_form(form, hasher);
            }
            Self::Post(form) => {
                hasher.write_tag("op_def.post");
                write_form(form, hasher);
            }
            Self::Delete((key, form)) => {
                hasher.write_tag("op_def.delete");
                key.write_semantic(hasher);
                write_form(form, hasher);
            }
        }
    }
}

impl SemanticHash for TCRef {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        match self {
            Self::Op(op) => {
                hasher.write_tag("ref.op");
                op.write_semantic(hasher);
            }
            Self::Id(id) => {
                hasher.write_tag("ref.id");
                hasher.write_bytes(id.to_string().as_bytes());
            }
            Self::Cond(cond) => {
                hasher.write_tag("ref.cond");
                cond.cond.write_semantic(hasher);
                cond.then.write_semantic(hasher);
                cond.or_else.write_semantic(hasher);
            }
            Self::After(after) => {
                hasher.write_tag("ref.after");
                after.when.write_semantic(hasher);
                after.then.write_semantic(hasher);
            }
            Self::While(while_ref) => {
                hasher.write_tag("ref.while");
                while_ref.cond.write_semantic(hasher);
                while_ref.closure.write_semantic(hasher);
                while_ref.state.write_semantic(hasher);
            }
            Self::ForEach(for_each) => {
                hasher.write_tag("ref.for_each");
                for_each.items.write_semantic(hasher);
                for_each.op.write_semantic(hasher);
                for_each.item_name.write_semantic(hasher);
            }
        }
    }
}

impl SemanticHash for Scalar {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        self.visit(&mut |node| {
            write_node(node, hasher);
            true
        });
    }
}

fn write_node(node: ScalarNode<'_>, hasher: &mut SemanticHasher) {
    match node {
        ScalarNode::Scalar(Scalar::Value(value)) => {
            hasher.write_tag("scalar.value");
            value.write_semantic(hasher);
        }
        ScalarNode::Scalar(Scalar::Ref(_)) => hasher.write_tag("scalar.ref"),
        ScalarNode::Scalar(Scalar::Op(op)) => {
            hasher.write_tag("scalar.op");
            match op {
                OpDef::Get((key, form)) => {
                    hasher.write_tag("op_def.get");
                    key.write_semantic(hasher);
                    hasher.write_len(form.len());
                }
                OpDef::Put((key, value, form)) => {
                    hasher.write_tag("op_def.put");
                    key.write_semantic(hasher);
                    value.write_semantic(hasher);
                    hasher.write_len(form.len());
                }
                OpDef::Post(form) => {
                    hasher.write_tag("op_def.post");
                    hasher.write_len(form.len());
                }
                OpDef::Delete((key, form)) => {
                    hasher.write_tag("op_def.delete");
                    key.write_semantic(hasher);
                    hasher.write_len(form.len());
                }
            }
        }
        ScalarNode::Scalar(Scalar::Map(map)) => {
            hasher.write_tag("scalar.map");
            hasher.write_tag("map");
            hasher.write_len(map.len());
        }
        ScalarNode::Scalar(Scalar::Tuple(tuple)) => {
            hasher.write_tag("scalar.tuple");
            hasher.write_len(tuple.len());
        }
        ScalarNode::Scalar(Scalar::Application(application)) => {
            hasher.write_tag("scalar.application");
            hasher.write_tag("application_definition");
            application.identity().write_semantic(hasher);
        }
        ScalarNode::Ref(reference) => match reference {
            TCRef::Op(_) => hasher.write_tag("ref.op"),
            TCRef::Id(id) => {
                hasher.write_tag("ref.id");
                hasher.write_bytes(id.to_string().as_bytes());
            }
            TCRef::Cond(_) => hasher.write_tag("ref.cond"),
            TCRef::After(_) => hasher.write_tag("ref.after"),
            TCRef::While(_) => hasher.write_tag("ref.while"),
            TCRef::ForEach(_) => hasher.write_tag("ref.for_each"),
        },
        ScalarNode::Op(op) => match op {
            OpRef::Get(_) => hasher.write_tag("op_ref.get"),
            OpRef::Put(_) => hasher.write_tag("op_ref.put"),
            OpRef::Post((_, params)) => {
                hasher.write_tag("op_ref.post");
                hasher.write_tag("map");
                hasher.write_len(params.len());
            }
            OpRef::Delete(_) => hasher.write_tag("op_ref.delete"),
        },
        ScalarNode::Subject(subject) => subject.write_semantic(hasher),
        ScalarNode::Binding(id) => id.write_semantic(hasher),
    }
}

impl SemanticHash for ApplicationDefinition<Scalar> {
    fn write_semantic(&self, hasher: &mut SemanticHasher) {
        hasher.write_tag("application_definition");
        self.identity().write_semantic(hasher);
        self.definition().write_semantic(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn application_digest_has_a_language_neutral_golden_value() {
        let input = futures::stream::iter([Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            include_bytes!("../fixtures/application_definition.json"),
        ))]);
        let definition: ApplicationDefinition<Scalar> =
            destream_json::try_decode((), input).await.expect("fixture");

        assert_eq!(
            definition.semantic_digest().hex(),
            "2f51a9553938a92fd406da3cd22102e0edd33cbe0c2500f3aa44770a95287d40"
        );
    }
}
