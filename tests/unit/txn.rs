use super::*;
use futures::TryStreamExt;

async fn encoded<'a>(value: impl en::IntoStream<'a> + 'a) -> Vec<u8> {
    destream_json::encode(value)
        .unwrap()
        .try_fold(Vec::new(), |mut bytes, chunk| async move {
            bytes.extend_from_slice(&chunk);
            Ok(bytes)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn txn_id_codecs_preserve_string_representation() {
    let ids = [
        TxnId::from_parts(NetworkTime::from_nanos(0), 0),
        TxnId::from_parts(NetworkTime::from_nanos(u64::MAX), u16::MAX).with_trace([0xab; 32]),
    ];
    for id in ids {
        let expected = format!("\"{id}\"").into_bytes();
        assert_eq!(encoded(id).await, expected);
        assert_eq!(encoded(&id).await, expected);
        let decoded: TxnId = destream_json::try_decode((), destream_json::encode(id).unwrap())
            .await
            .unwrap();
        assert_eq!(decoded, id);
    }

    let values = (
        Some(ids[1]),
        None::<TxnId>,
        ids.to_vec(),
        std::collections::BTreeMap::from([(ids[1], ids[0])]),
    );
    let decoded: (
        Option<TxnId>,
        Option<TxnId>,
        Vec<TxnId>,
        std::collections::BTreeMap<TxnId, TxnId>,
    ) = destream_json::try_decode((), destream_json::encode(values.clone()).unwrap())
        .await
        .unwrap();
    assert_eq!(decoded, values);

    let uppercase = ids[1].to_string().to_uppercase();
    let decoded: TxnId = destream_json::try_decode((), destream_json::encode(uppercase).unwrap())
        .await
        .unwrap();
    assert_eq!(decoded, ids[1]);
}

#[tokio::test]
async fn txn_id_codecs_reject_malformed_input_without_panicking() {
    let trace = "0".repeat(64);
    for text in [
        String::new(),
        "1".into(),
        "1-2".into(),
        format!("1-2-{trace}-extra"),
        format!("18446744073709551616-2-{trace}"),
        format!("1-65536-{trace}"),
        "1-2-00".into(),
        format!("1-2-{trace}0"),
        format!("1-2-{}", "g".repeat(64)),
        format!("1-2-a€{}", "0".repeat(60)),
    ] {
        assert!(text.parse::<TxnId>().is_err(), "{text}");
        let decoded: Result<TxnId, _> =
            destream_json::try_decode((), destream_json::encode(text).unwrap()).await;
        assert!(decoded.is_err());
    }

    for json in ["null", "123", "true", "[]", "{}"] {
        let stream = futures::stream::iter([Ok::<_, std::io::Error>(bytes::Bytes::from(json))]);
        let decoded: Result<TxnId, _> = destream_json::try_decode((), stream).await;
        assert!(decoded.is_err(), "{json}");
    }
}
