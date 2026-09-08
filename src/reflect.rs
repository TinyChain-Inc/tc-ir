use std::str::FromStr;

use pathlink::Link;
use tc_error::{TCError, TCResult};
use tc_value::Value;

use crate::{Id, NativeClass, OpDef, OpRef, Scalar, TCRef};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reflection {
    ScalarClass,
    ScalarRefParts,
    OpDefForm,
    OpDefLastId,
    OpDefScalars,
}

impl Reflection {
    pub fn from_segments(path: &[pathlink::PathSegment]) -> Option<Self> {
        let path = path
            .first()
            .is_some_and(|segment| segment.as_str() == "state")
            .then(|| &path[1..])
            .unwrap_or(path);
        let segments = path
            .iter()
            .map(pathlink::PathSegment::as_str)
            .collect::<Vec<_>>();
        match segments.as_slice() {
            ["scalar", "reflect", "class"] => Some(Self::ScalarClass),
            ["scalar", "reflect", "ref_parts"] => Some(Self::ScalarRefParts),
            ["scalar", "op", "reflect", "form"] => Some(Self::OpDefForm),
            ["scalar", "op", "reflect", "last_id"] => Some(Self::OpDefLastId),
            ["scalar", "op", "reflect", "scalars"] => Some(Self::OpDefScalars),
            _ => None,
        }
    }

    pub fn apply(self, scalar: &Scalar) -> TCResult<Scalar> {
        Ok(match self {
            Self::ScalarClass => Scalar::Value(Value::Link(match scalar {
                Scalar::Value(value) => class_link(value.class().path()),
                Scalar::Op(op) => class_from_opdef(op),
                Scalar::Ref(reference) => class_from_tcref(reference),
                Scalar::Map(_) => class_link(crate::SCALAR_MAP),
                Scalar::Tuple(_) => class_link(crate::SCALAR_TUPLE),
                Scalar::Application(definition) => definition.identity().link(),
            })),
            Self::ScalarRefParts => Scalar::Tuple(match scalar {
                Scalar::Ref(reference) => match reference.as_ref() {
                    TCRef::Cond(value) => vec![
                        Scalar::from(value.cond.clone()),
                        value.then.clone(),
                        value.or_else.clone(),
                    ],
                    TCRef::After(value) => vec![value.when.clone(), value.then.clone()],
                    TCRef::While(value) => vec![
                        value.cond.clone(),
                        value.closure.clone(),
                        value.state.clone(),
                    ],
                    TCRef::ForEach(value) => vec![
                        value.items.clone(),
                        value.op.clone(),
                        Scalar::Value(Value::String(value.item_name.to_string())),
                    ],
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            }),
            Self::OpDefForm => Scalar::Tuple(
                opdef(scalar)?
                    .form()
                    .iter()
                    .map(|(id, scalar)| {
                        Scalar::Tuple(vec![
                            Scalar::Value(Value::String(id.to_string())),
                            scalar.clone(),
                        ])
                    })
                    .collect(),
            ),
            Self::OpDefLastId => Scalar::Value(
                opdef(scalar)?
                    .last_id()
                    .map(|id| Value::String(id.to_string()))
                    .unwrap_or(Value::None),
            ),
            Self::OpDefScalars => Scalar::Tuple(children(scalar)),
        })
    }
}

fn children(scalar: &Scalar) -> Vec<Scalar> {
    match scalar {
        Scalar::Value(_) => Vec::new(),
        Scalar::Map(map) => map.values().cloned().collect(),
        Scalar::Tuple(tuple) => tuple.clone(),
        Scalar::Op(op) => op.form().iter().map(|(_, scalar)| scalar.clone()).collect(),
        Scalar::Application(definition) => vec![definition.definition().clone()],
        Scalar::Ref(reference) => match reference.as_ref() {
            TCRef::Id(_) => Vec::new(),
            TCRef::Op(op) => match op {
                OpRef::Get((_, key)) | OpRef::Delete((_, key)) => vec![key.clone()],
                OpRef::Put((_, key, value)) => vec![key.clone(), value.clone()],
                OpRef::Post((_, params)) => {
                    params.iter().map(|(_, scalar)| scalar.clone()).collect()
                }
            },
            TCRef::Cond(value) => vec![
                Scalar::from(value.cond.clone()),
                value.then.clone(),
                value.or_else.clone(),
            ],
            TCRef::After(value) => vec![value.when.clone(), value.then.clone()],
            TCRef::While(value) => vec![
                value.cond.clone(),
                value.closure.clone(),
                value.state.clone(),
            ],
            TCRef::ForEach(value) => vec![value.items.clone(), value.op.clone()],
        },
    }
}

fn opdef(scalar: &Scalar) -> TCResult<&OpDef> {
    match scalar {
        Scalar::Op(opdef) => Ok(opdef),
        _ => Err(TCError::bad_request("expected OpDef scalar parameter")),
    }
}

fn class_link(path: impl Into<pathlink::PathBuf>) -> Link {
    Link::from_str(&path.into().to_string()).expect("IR class link")
}

fn class_from_opdef(op: &OpDef) -> Link {
    class_link(match op {
        OpDef::Get(_) => crate::OPDEF_GET,
        OpDef::Put(_) => crate::OPDEF_PUT,
        OpDef::Post(_) => crate::OPDEF_POST,
        OpDef::Delete(_) => crate::OPDEF_DELETE,
    })
}

fn class_from_tcref(reference: &TCRef) -> Link {
    class_link(match reference {
        TCRef::Cond(_) => crate::TCREF_COND,
        TCRef::After(_) => crate::TCREF_AFTER,
        TCRef::While(_) => crate::TCREF_WHILE,
        TCRef::ForEach(_) => crate::TCREF_FOR_EACH,
        TCRef::Id(_) => crate::SCALAR_REF_PREFIX,
        TCRef::Op(op) => match op {
            OpRef::Get(_) => crate::OPREF_GET,
            OpRef::Put(_) => crate::OPREF_PUT,
            OpRef::Post(_) => crate::OPREF_POST,
            OpRef::Delete(_) => crate::OPREF_DELETE,
        },
    })
}

pub fn reflection_param(params: &crate::Map<Scalar>) -> TCResult<Scalar> {
    let scalar = "scalar".parse::<Id>().expect("static parameter id");
    let op = "op".parse::<Id>().expect("static parameter id");
    params
        .get(&scalar)
        .or_else(|| params.get(&op))
        .cloned()
        .ok_or_else(|| TCError::bad_request("missing scalar parameter"))
}

#[cfg(test)]
mod tests {
    use tc_value::Value;

    use super::*;

    #[test]
    fn scalar_children_are_reflected_without_an_opdef_wrapper() {
        let leaf = Scalar::Value(Value::String("leaf".into()));
        assert_eq!(
            Reflection::OpDefScalars.apply(&leaf).unwrap(),
            Scalar::Tuple(Vec::new())
        );
        let tuple = Scalar::Tuple(vec![leaf.clone()]);
        assert_eq!(
            Reflection::OpDefScalars.apply(&tuple).unwrap(),
            Scalar::Tuple(vec![leaf])
        );
    }
}
