use super::*;

#[test]
fn sibling_count_does_not_expand_event_continuations() {
    for count in [1, 4097] {
        let form = || {
            (0..count)
                .map(|n| (format!("item{n}").parse().unwrap(), Scalar::default()))
                .collect::<Vec<_>>()
        };
        for value in [
            Scalar::Tuple(vec![Scalar::default(); count]),
            Scalar::Map(form().into_iter().collect()),
            Scalar::from(OpDef::Post(form())),
        ] {
            for root in [Node::BorrowedScalar(&value), Node::Scalar(value.clone())] {
                let mut events = Events::new::<destream_json::en::Error>(root).unwrap();
                while events
                    .next_event::<destream_json::en::Error>()
                    .unwrap()
                    .is_some()
                {
                    assert!(events.pending.len() <= 8);
                }
            }
        }
    }
}
