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
        include_str!("../../fixtures/lexical/valid-forward.json"),
        include_str!("../../fixtures/lexical/valid-capture.json"),
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
        include_str!("../../fixtures/lexical/invalid-shadow.json"),
        include_str!("../../fixtures/lexical/invalid-foreach.json"),
        include_str!("../../fixtures/lexical/invalid-while.json"),
    ] {
        let source = stream::once(future::ready(Ok::<_, std::convert::Infallible>(
            Bytes::copy_from_slice(fixture.as_bytes()),
        )));
        let decoded: Result<OpDef, _> = destream_json::try_decode((), source).await;
        decoded.expect_err("invalid lexical fixture");
    }
}
