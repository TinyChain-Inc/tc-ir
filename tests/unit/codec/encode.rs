use futures::{StreamExt, TryStreamExt};

use crate::{OpDef, Scalar};
use tc_value::Value;

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
