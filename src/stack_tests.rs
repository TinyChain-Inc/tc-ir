use std::sync::Arc;

use async_hash::{Hash, Sha256};
use bytes::Bytes;
use futures::{stream, StreamExt, TryStreamExt};
use get_size::GetSize;

use crate::{Cond, Scalar, TCRef};

async fn decode(binary: bool, input: Vec<u8>) -> Result<Scalar, String> {
    let source = stream::iter(input.chunks(17).map(Bytes::copy_from_slice));
    if binary {
        tbon::de::decode((), source)
            .await
            .map_err(|e| e.to_string())
    } else {
        destream_json::decode((), source)
            .await
            .map_err(|e| e.to_string())
    }
}

fn wire(binary: bool, map: bool, depth: usize) -> Vec<u8> {
    let prefix = if map {
        if binary {
            "{\"item\""
        } else {
            "{\"item\":"
        }
    } else {
        "["
    };
    let suffix = if map { "}" } else { "]" };
    format!(
        "{}{}{}",
        prefix.repeat(depth),
        if binary { "\x01" } else { "null" },
        suffix.repeat(depth)
    )
    .into_bytes()
}

fn assert_depth(value: &Scalar, map: bool, depth: usize) {
    let mut next = value;
    for _ in 0..depth {
        next = match &next {
            Scalar::Map(values) if map => values.values().next().unwrap(),
            Scalar::Tuple(values) if !map => &values[0],
            _ => panic!("unexpected scalar shape"),
        };
    }
    assert!(matches!(&next,Scalar::Value(value) if matches!(&value,tc_value::Value::None)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_depth_and_lifecycle_use_the_worker_stack() {
    tokio::spawn(async {
        for binary in [false, true] {
            for map in [false, true] {
                for depth in [2, 64, 65] {
                    let value = decode(binary, wire(binary, map, depth)).await;
                    if depth == 65 {
                        assert!(value.is_err());
                        continue;
                    }
                    let value = value.unwrap();
                    assert_depth(&value, map, depth);
                    let cloned = value.clone();
                    assert_eq!(value, cloned);
                    assert_eq!(Hash::<Sha256>::hash(&value), Hash::<Sha256>::hash(&cloned));
                    assert!(value.get_heap_size() > 0);
                    assert!(!format!("{value:?}").is_empty());
                    let mut encoded = destream_json::encode(&value).unwrap();
                    let mut bytes = Vec::new();
                    while let Some(chunk) = encoded.try_next().await.unwrap() {
                        bytes.extend_from_slice(&chunk);
                    }
                    drop(encoded);
                    assert_eq!(decode(false, bytes).await.unwrap(), value);
                    drop(cloned);
                    drop(value);
                }
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_and_cancelled_deep_inputs_release_owned_state() {
    tokio::spawn(async {
        for binary in [false, true] {
            let mut complete = wire(binary, false, 63);
            complete.insert(0, b'[');
            if !binary {
                complete.push(b',');
            }
            for prefix in [vec![b'['; 64], complete] {
                let mut malformed = prefix.clone();
                malformed.push(b'}');
                assert!(decode(binary, malformed).await.is_err());
                let source = stream::iter([
                    Ok(Bytes::from(prefix.clone())),
                    Err(std::io::Error::other("injected source failure")),
                ]);
                let error = if binary {
                    tbon::de::try_decode::<_, _, Scalar>((), source)
                        .await
                        .err()
                        .unwrap()
                        .to_string()
                } else {
                    destream_json::try_decode::<_, _, Scalar>((), source)
                        .await
                        .err()
                        .unwrap()
                        .to_string()
                };
                assert!(error.contains("injected source failure"));
                for poll in [false, true] {
                    let owner = Arc::new(());
                    let held = Arc::clone(&owner);
                    let source =
                        stream::iter([Ok::<_, std::io::Error>(Bytes::from(prefix.clone()))])
                            .chain(stream::pending())
                            .inspect(move |_| {
                                let _ = &held;
                            });
                    let mut future = Box::pin(async move {
                        if binary {
                            tbon::de::try_decode::<_, _, Scalar>((), source)
                                .await
                                .map_err(|e| e.to_string())
                        } else {
                            destream_json::try_decode::<_, _, Scalar>((), source)
                                .await
                                .map_err(|e| e.to_string())
                        }
                    });
                    if poll {
                        assert!(futures::poll!(&mut future).is_pending());
                    }
                    drop(future);
                    assert_eq!(Arc::strong_count(&owner), 1);
                }
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wide_encoding_and_stream_drop_do_not_build_call_chains() {
    tokio::spawn(async {
        for value in [
            Scalar::Tuple((0..65536).map(|_| Scalar::default()).collect()),
            Scalar::Map(
                (0..8192)
                    .map(|i| (format!("item{i}").parse().unwrap(), Scalar::default()))
                    .collect(),
            ),
        ] {
            let mut required = std::collections::BTreeSet::new();
            value.requires(&mut required);
            assert!(required.is_empty());
            assert!(!value.is_ref());
            assert!(value.get_heap_size() > 0);
            assert_eq!(
                <tc_value::Value as safecast::TryCastFrom<Scalar>>::can_cast_from(&value),
                matches!(&value, Scalar::Tuple(_))
            );
            let mut stream = destream_json::encode(value.clone()).unwrap();
            assert!(stream.try_next().await.unwrap().is_some());
            drop(stream);
            drop(destream_json::encode(value.clone()).unwrap());
            let encoded = destream_json::encode(value.clone()).unwrap();
            let decoded: Scalar = destream_json::try_decode((), encoded).await.unwrap();
            assert_eq!(value, decoded);
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_operation_forms_keep_wire_and_hash_identity() {
    tokio::spawn(async {
        let mut scalar = Scalar::default();
        for depth in 0..32 {
            scalar = if depth % 2 == 0 {
                crate::OpDef::Post(vec![(format!("result{depth}").parse().unwrap(), scalar)]).into()
            } else {
                TCRef::After(Box::new(crate::After::new(Scalar::default(), scalar))).into()
            };
        }
        for binary in [false, true] {
            let encoded = if binary {
                tbon::en::encode(&scalar)
                    .unwrap()
                    .map_ok(|bytes| bytes.to_vec())
                    .try_concat()
                    .await
                    .unwrap()
            } else {
                destream_json::encode(&scalar)
                    .unwrap()
                    .map_ok(|bytes| bytes.to_vec())
                    .try_concat()
                    .await
                    .unwrap()
            };
            let decoded = decode(binary, encoded).await.unwrap();
            assert_eq!(scalar, decoded);
            assert_eq!(
                Hash::<Sha256>::hash(&scalar),
                Hash::<Sha256>::hash(&decoded)
            );
        }
        let empty = Scalar::Tuple(Vec::new());
        assert_eq!(
            Hash::<Sha256>::hash(&empty),
            async_hash::default_hash::<Sha256>()
        );
        let a = Scalar::from(1_u64);
        let b = Scalar::from(2_u64);
        let reference = TCRef::After(Box::new(crate::After::new(a.clone(), b.clone())));
        assert_eq!(
            Hash::<Sha256>::hash(&reference),
            Hash::<Sha256>::hash((&a, &b))
        );
        let subject = crate::Subject::Link("/service/test/test/1.0.0".parse().unwrap());
        let call = crate::OpRef::Put((subject.clone(), a.clone(), b.clone()));
        assert_eq!(
            Hash::<Sha256>::hash(&call),
            Hash::<Sha256>::hash((&subject, &a, &b))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn operation_argument_maps_and_reference_tails_preserve_their_contracts() {
    let scalar = decode(false, br#"{"$f":{"x":1,"x":2}}"#.to_vec())
        .await
        .unwrap();
    let Scalar::Ref(reference) = &scalar else {
        panic!("call reference")
    };
    let TCRef::Op(crate::OpRef::Post((_, params))) = reference.as_ref() else {
        panic!("POST arguments")
    };
    assert_eq!(params.get("x"), Some(&Scalar::from(2_u64)));

    let wire = br#"{"$f":[null],"extra":17}"#;
    let source = stream::iter([Bytes::copy_from_slice(wire)]);
    let call: crate::OpRef = destream_json::decode((), source).await.unwrap();
    assert!(matches!(call, crate::OpRef::Get(_)));
    assert!(decode(false, wire.to_vec()).await.is_err());
}

#[test]
fn value_conversion_checks_combined_tuple_depth_before_construction() {
    use safecast::TryCastFrom;
    use tc_value::{Value, MAX_VALUE_DEPTH};

    for (scalar_depth, value_depth, accepted) in [
        (0, MAX_VALUE_DEPTH, true),
        (MAX_VALUE_DEPTH, 0, true),
        (32, 32, true),
        (32, 33, false),
        (0, MAX_VALUE_DEPTH + 1, false),
        (MAX_VALUE_DEPTH + 1, 0, false),
    ] {
        let mut value = Value::from(7_u64);
        for _ in 0..value_depth {
            value = Value::Tuple(vec![value]);
        }
        let mut scalar = Scalar::from(value);
        for _ in 0..scalar_depth {
            scalar = Scalar::Tuple(vec![scalar]);
        }

        assert_eq!(
            Value::can_cast_from(&scalar),
            accepted,
            "Scalar depth {scalar_depth}, Value depth {value_depth}"
        );
        let value = Value::opt_cast_from(scalar);
        assert_eq!(value.is_some(), accepted);
        if let Some(value) = value {
            assert!(value.within_depth(MAX_VALUE_DEPTH));
            assert_eq!(value, value.clone());
        }
    }

    let mut empty = Scalar::Tuple(Vec::new());
    for _ in 1..MAX_VALUE_DEPTH {
        empty = Scalar::Tuple(vec![empty]);
    }
    assert!(Value::can_cast_from(&empty));
    assert!(Value::opt_cast_from(Scalar::Tuple(vec![empty])).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_codec_nesting_uses_one_structural_allowance() {
    tokio::spawn(async {
        for binary in [false, true] {
            for depth in [32, 33] {
                // Typed Values remain Values instead of decoding as Scalar tuples.
                let value_wire = format!(
                    "{{\"/state/scalar/value/tuple\":{}}}",
                    String::from_utf8(wire(false, false, 32)).unwrap()
                );
                let json = format!("{}{}{}", "[".repeat(depth), value_wire, "]".repeat(depth));
                let bytes = if binary {
                    // The TBON type map differs only in its key/value separator here.
                    json.replace("\":", "\"")
                        .replace("null", "\u{1}")
                        .into_bytes()
                } else {
                    json.into_bytes()
                };
                assert_eq!(decode(binary, bytes).await.is_ok(), depth == 32);
            }
            for depth in [64, 65] {
                let mut reference = TCRef::Id("$condition".parse().unwrap());
                for _ in 0..depth {
                    reference = TCRef::Cond(Box::new(Cond::new(
                        reference,
                        Scalar::default(),
                        Scalar::default(),
                    )));
                }
                let bytes = if binary {
                    tbon::en::encode(&reference)
                        .unwrap()
                        .map_ok(|chunk| chunk.to_vec())
                        .try_concat()
                        .await
                        .unwrap()
                } else {
                    destream_json::encode(&reference)
                        .unwrap()
                        .map_ok(|chunk| chunk.to_vec())
                        .try_concat()
                        .await
                        .unwrap()
                };
                let source = stream::iter([Bytes::from(bytes)]);
                let result = if binary {
                    tbon::de::decode::<_, TCRef>((), source)
                        .await
                        .map_err(|error| error.to_string())
                } else {
                    destream_json::decode::<_, TCRef>((), source)
                        .await
                        .map_err(|error| error.to_string())
                };
                assert_eq!(
                    result.is_ok(),
                    depth == 64,
                    "reference depth {depth}: {result:?}"
                );
                if depth == 64 {
                    let decoded = result.unwrap();
                    assert_eq!(decoded, reference);
                    let cloned = decoded.clone();
                    assert_eq!(cloned, decoded);
                    assert_eq!(
                        Hash::<Sha256>::hash(&cloned),
                        Hash::<Sha256>::hash(&decoded)
                    );
                    assert!(!format!("{decoded:?}").is_empty());
                    drop(cloned);
                    drop(decoded);
                }
            }
        }
    })
    .await
    .unwrap();
}

#[test]
fn streamed_container_hashes_preserve_native_boundaries() {
    use crate::{Id, Map, OpDef};
    use tc_value::Value;
    for width in [0, 1, 4097] {
        let items: Vec<Scalar> = (0..width)
            .map(|i| Scalar::from(Value::from(i as u64)))
            .collect();
        let expected = Hash::<Sha256>::hash(&items);
        let tuple = Scalar::Tuple(items);
        assert_eq!(Hash::<Sha256>::hash(&tuple), expected);
        assert_eq!(tuple.clone(), tuple);
        let form: Vec<(Id, Scalar)> = (0..width)
            .map(|i| {
                (
                    format!("k{i}").parse().unwrap(),
                    Scalar::from(Value::from(i as u64)),
                )
            })
            .collect();
        let expected = Hash::<Sha256>::hash(&form);
        let op = Scalar::Op(OpDef::Post(form.clone()));
        assert_eq!(Hash::<Sha256>::hash(&op), expected);
        assert_eq!(op.clone(), op);
        let map: Map<Scalar> = form.into_iter().collect();
        let expected = Hash::<Sha256>::hash(&map);
        let scalar = Scalar::Map(map);
        assert_eq!(Hash::<Sha256>::hash(&scalar), expected);
        assert_eq!(scalar.clone(), scalar);
    }
}
