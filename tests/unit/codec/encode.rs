use destream::en::IntoStream;
use futures::{StreamExt, TryStreamExt};
use get_size::GetSize;
use pathlink::path_label;

use crate::{IdRef, OpDef, OpRef, Scalar, Subject, TCRef};
use tc_value::Value;

async fn json<'en>(value: impl IntoStream<'en> + 'en) -> Vec<u8> {
    destream_json::encode(value)
        .unwrap()
        .map_ok(|chunk| chunk.to_vec())
        .try_concat()
        .await
        .unwrap()
}

async fn tbon<'en>(value: impl IntoStream<'en> + 'en) -> Vec<u8> {
    tbon::en::encode(value)
        .unwrap()
        .map_ok(|chunk| chunk.to_vec())
        .try_concat()
        .await
        .unwrap()
}

#[tokio::test]
async fn reserved_type_subjects_use_unambiguous_invocation_forms() {
    for subject in [
        "/state/scalar/value/string",
        "/state/scalar/op/post",
        "/state/scalar/ref/cond",
        "/state/collection/tensor",
        "https://example.com/state/scalar/value/string",
        "/lib/example/code/1.0.0/run",
    ] {
        let subject = Subject::Link(subject.parse().unwrap());
        for op in [
            OpRef::Get((subject.clone(), Scalar::default())),
            OpRef::Put((
                subject.clone(),
                Scalar::default(),
                Value::from("https://example.com/").into(),
            )),
            OpRef::Post((subject.clone(), crate::Map::new())),
            OpRef::Delete((subject, Scalar::default())),
        ] {
            for binary in [false, true] {
                let (owned, borrowed) = if binary {
                    (
                        tbon::en::encode(op.clone()).unwrap().boxed(),
                        tbon::en::encode(&op).unwrap().boxed(),
                    )
                } else {
                    // The codecs' Error types differ; normalize only this
                    // test's byte-stream error representation.
                    let owned = destream_json::encode(op.clone())
                        .unwrap()
                        .map_err(|e| e.to_string())
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap();

                    let borrowed = destream_json::encode(&op)
                        .unwrap()
                        .map_err(|e| e.to_string())
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap();

                    assert_eq!(owned.concat(), borrowed.concat());
                    let decoded: Scalar = destream_json::try_decode(
                        (),
                        futures::stream::iter(owned.into_iter().map(Ok::<_, std::io::Error>)),
                    )
                    .await
                    .unwrap();

                    assert_eq!(decoded, Scalar::from(TCRef::Op(op.clone())));
                    continue;
                };

                let owned = owned.try_collect::<Vec<_>>().await.unwrap();
                let borrowed = borrowed.try_collect::<Vec<_>>().await.unwrap();
                assert_eq!(owned.concat(), borrowed.concat());
                let decoded: Scalar = tbon::de::try_decode(
                    (),
                    futures::stream::iter(owned.into_iter().map(Ok::<_, std::io::Error>)),
                )
                .await
                .unwrap();

                assert_eq!(decoded, Scalar::from(TCRef::Op(op.clone())));
            }
        }
    }
}

#[tokio::test]
async fn native_values_preserve_variants_and_reopen_footprints() {
    let link = Value::Link(crate::OPDEF_POST.to_string().parse().unwrap());
    let value_tuple = Value::Tuple(vec![Value::from("text"), Value::Tuple(vec![link.clone()])]);
    let mut values = vec![
        Scalar::Value(value_tuple.clone()),
        Scalar::Tuple(vec![Scalar::Value(value_tuple)]),
        Scalar::Value(link),
        Scalar::Tuple(vec![Scalar::Value(Value::String("text".to_owned()))]),
    ];

    for count in [0, 1, 2, 3, 4, 5, 8, 9, 17] {
        values.push(Scalar::Tuple(vec![Scalar::default(); count]));
        if count > 0 {
            values.push(Scalar::from(OpDef::Post(
                (0..count)
                    .map(|n| (format!("item{n}").parse().unwrap(), Scalar::default()))
                    .collect(),
            )));
        }
    }

    for value in values {
        for binary in [false, true] {
            let (borrowed, owned) = if binary {
                (tbon(&value).await, tbon(value.clone()).await)
            } else {
                (json(&value).await, json(value.clone()).await)
            };

            assert_eq!(borrowed, owned);
            let input = futures::stream::iter([bytes::Bytes::from(borrowed)]);
            let decoded: Scalar = if binary {
                tbon::de::decode((), input).await.unwrap()
            } else {
                destream_json::decode((), input).await.unwrap()
            };

            assert_eq!(decoded, value);
            assert!(decoded.get_size() <= value.get_size());
        }
    }
}

#[tokio::test]
async fn native_tags_and_subjects_preserve_wire_strings() {
    let label = path_label(&["literal%name", "λ"]);
    for wire in [
        json(label).await,
        json(&label).await,
        tbon(label).await,
        tbon(&label).await,
    ] {
        assert_eq!(wire, "\"/literal%name/λ\"".as_bytes());
    }

    for subject in [
        Subject::Link("/literal%25name/%CE%BB".parse().unwrap()),
        Subject::Ref(
            IdRef::new("λ".parse().unwrap()),
            "/literal%name".parse().unwrap(),
        ),
    ] {
        let expected = format!("\"{subject}\"");
        assert_eq!(json(&subject).await, expected.as_bytes());
        assert_eq!(json(subject.clone()).await, expected.as_bytes());
        assert_eq!(tbon(&subject).await, expected.as_bytes());
        assert_eq!(tbon(subject.clone()).await, expected.as_bytes());
        let op = OpRef::Delete((subject.clone(), Scalar::default()));
        let expected = format!("{{\"/state/scalar/ref/op/delete\":[\"{subject}\",null]}}");
        assert_eq!(json(&op).await, expected.as_bytes());
        assert_eq!(json(op.clone()).await, expected.as_bytes());
        let encoded = tbon::en::encode(&op).unwrap();
        let decoded: OpRef = tbon::de::try_decode((), encoded).await.unwrap();
        assert_eq!(decoded, op);
        assert_eq!(tbon(op.clone()).await, tbon(&op).await);
    }
}

#[tokio::test]
async fn sibling_count_does_not_expand_event_continuations() {
    for count in [1, 4097] {
        let form = || {
            (0..count)
                .map(|n| (format!("item{n}").parse().unwrap(), Scalar::default()))
                .collect::<Vec<_>>()
        };
        for mut value in [
            Scalar::Tuple(vec![Scalar::default(); count]),
            Scalar::Map(form().into_iter().collect()),
            Scalar::from(OpDef::Post(form())),
        ] {
            for binary in [false, true] {
                let (borrowed, owned) = if binary {
                    (
                        tbon::en::encode(&value)
                            .unwrap()
                            .try_collect::<Vec<_>>()
                            .await
                            .unwrap(),
                        tbon::en::encode(value.clone())
                            .unwrap()
                            .try_collect::<Vec<_>>()
                            .await
                            .unwrap(),
                    )
                } else {
                    (
                        destream_json::encode(&value)
                            .unwrap()
                            .try_collect::<Vec<_>>()
                            .await
                            .unwrap(),
                        destream_json::encode(value.clone())
                            .unwrap()
                            .try_collect::<Vec<_>>()
                            .await
                            .unwrap(),
                    )
                };
                assert_eq!(borrowed.concat(), owned.concat());
            }

            // JSON rejects NaN when its child encoder is constructed. The error
            // must follow every preceding sibling, not merely the first delimiter.
            let last = match &mut value {
                Scalar::Tuple(values) => values.last_mut().unwrap(),
                Scalar::Map(values) => values.values_mut().next_back().unwrap(),
                Scalar::Op(OpDef::Post(form)) => &mut form.last_mut().unwrap().1,
                _ => unreachable!(),
            };
            *last = Scalar::Value(Value::Number(f64::NAN.into()));

            for owned in [false, true] {
                let mut encoded = if owned {
                    destream_json::encode(value.clone()).unwrap().boxed()
                } else {
                    destream_json::encode(&value).unwrap().boxed()
                };
                let mut prefix = Vec::new();
                loop {
                    match encoded.try_next().await {
                        Ok(Some(bytes)) => prefix.extend_from_slice(&bytes),
                        Ok(None) => panic!("expected the final NaN to fail encoding"),
                        Err(error) => {
                            assert!(error.to_string().contains("floating-point"));
                            break;
                        }
                    }
                }
                assert_eq!(
                    String::from_utf8(prefix).unwrap().matches("null").count(),
                    count - 1
                );
            }
        }
    }
}
