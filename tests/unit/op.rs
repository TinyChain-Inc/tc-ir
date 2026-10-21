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

#[test]
fn recursive_visitors_preserve_order_and_scope() {
    let link = pathlink::Link::from(pathlink::PathBuf::from(pathlink::path_label(&["test"])));
    let call = |name: &str| {
        Scalar::from(TCRef::Op(OpRef::Get((
            Subject::Link(link.clone().append(id(name))),
            Scalar::default(),
        ))))
    };
    let value = Scalar::from(TCRef::Op(OpRef::Put((
        Subject::Link(link.clone()),
        call("key"),
        call("value"),
    ))));
    let mut methods = Vec::new();
    value.visit_referenced_methods(&mut |link, method| methods.push((link.clone(), method)));
    assert_eq!(
        methods,
        vec![
            (link.clone().append(id("key")), crate::Method::Get),
            (link.clone().append(id("value")), crate::Method::Get),
            (link.clone(), crate::Method::Put),
        ]
    );

    let mut op = OpDef::Post(vec![(id("first"), value), (id("last"), call("last"))]);
    let mut subjects = Vec::new();
    op.map_subjects(&mut |subject| subjects.push(subject.clone()));
    assert_eq!(
        subjects,
        vec![
            Subject::Link(link.clone().append(id("last"))),
            Subject::Link(link.clone()),
            Subject::Link(link.clone().append(id("value"))),
            Subject::Link(link.append(id("key"))),
        ]
    );
    assert!(!Scalar::Op(op.clone()).is_ref());
    assert!(Scalar::Tuple(vec![Scalar::Op(op), reference("$input")]).is_ref());

    let each = Scalar::from(TCRef::ForEach(Box::new(crate::ForEach::new(
        reference("$item"),
        reference("$item"),
        id("item"),
    ))));
    let while_ref = Scalar::from(TCRef::While(Box::new(crate::While::new(
        reference("$state"),
        reference("$state"),
        reference("$state"),
    ))));
    let mut required = BTreeSet::new();
    Scalar::Tuple(vec![each, while_ref, reference("$self")]).requires(&mut required);
    assert_eq!(required, BTreeSet::from([id("item"), id("state")]));

    // Body validation precedes items; binding errors precede either child.
    let invalid = |name: &str| {
        Scalar::Op(OpDef::Post(vec![
            (id(name), Scalar::default()),
            (id(name), Scalar::default()),
        ]))
    };
    let each = Scalar::from(TCRef::ForEach(Box::new(crate::ForEach::new(
        invalid("items"),
        invalid("body"),
        id("item"),
    ))));
    let op = OpDef::Post(vec![(id("result"), each.clone())]);
    assert!(op
        .validate()
        .unwrap_err()
        .to_string()
        .contains("duplicate OpDef binding $body"));
    let op = OpDef::Post(vec![(id("item"), each)]);
    assert!(op
        .validate()
        .unwrap_err()
        .to_string()
        .contains("shadowed OpDef binding $item"));
}
