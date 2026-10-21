use destream::en::{self, IntoStream, ToStream};
use futures::stream;

use crate::{Id, Map, OpDef, OpRef, Scalar, TCRef};
use tc_value::Value;

enum Leaf<'a> {
    Value(Value),
    BorrowedValue(&'a Value),
    Text(String),
}

impl<'a> IntoStream<'a> for Leaf<'a> {
    fn into_stream<E: en::Encoder<'a>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self {
            Self::Value(value) => value.into_stream(encoder),
            Self::BorrowedValue(value) => value.to_stream(encoder),
            Self::Text(value) => value.into_stream(encoder),
        }
    }
}

enum Node<'a> {
    Scalar(Scalar),
    Ref(TCRef),
    Op(OpDef),
    Call(OpRef),
    Map(Map<Scalar>),
    Form(Vec<(Id, Scalar)>),
    TupleItems(std::vec::IntoIter<Scalar>),
    MapItems(<Map<Scalar> as IntoIterator>::IntoIter),
    FormItems(std::vec::IntoIter<(Id, Scalar)>),
    BorrowedScalar(&'a Scalar),
    BorrowedRef(&'a TCRef),
    BorrowedOp(&'a OpDef),
    BorrowedCall(&'a OpRef),
    BorrowedMap(&'a Map<Scalar>),
    BorrowedForm(&'a [(Id, Scalar)]),
    BorrowedTupleItems(std::slice::Iter<'a, Scalar>),
    BorrowedMapItems(<&'a Map<Scalar> as IntoIterator>::IntoIter),
    BorrowedFormItems(std::slice::Iter<'a, (Id, Scalar)>),
    SeqStart(usize),
    MapStart(usize),
    End,
    Leaf(Leaf<'a>),
}

struct Events<'a> {
    pending: Vec<Node<'a>>,
}

impl<'a> Events<'a> {
    fn new<E: en::Error>(root: Node<'a>) -> Result<Self, E> {
        let mut pending = Vec::new();
        pending.try_reserve(1).map_err(E::custom)?;
        pending.push(root);

        Ok(Self { pending })
    }

    fn tag(&mut self, key: String) {
        self.pending
            .extend([Node::Leaf(Leaf::Text(key)), Node::MapStart(1)]);
    }

    fn text(&mut self, value: impl ToString) {
        self.pending.push(Node::Leaf(Leaf::Text(value.to_string())));
    }

    fn fields<const N: usize>(&mut self, values: [Node<'a>; N]) {
        self.pending.push(Node::End);
        self.pending.extend(values.into_iter().rev());
        self.pending.push(Node::SeqStart(N));
    }

    fn next_event<E: en::Error>(&mut self) -> Result<Option<en::Event<Leaf<'a>>>, E> {
        loop {
            let Some(node) = self.pending.pop() else {
                return Ok(None);
            };
            // Container iterators retain siblings in their native storage. Every
            // expansion adds at most eight nodes, independent of container width.
            let additional = match &node {
                Node::SeqStart(_) | Node::MapStart(_) | Node::End | Node::Leaf(_) => 0,
                _ => 8,
            };
            self.pending.try_reserve(additional).map_err(E::custom)?;

            match node {
                Node::SeqStart(len) => return Ok(Some(en::Event::SeqStart(Some(len)))),
                Node::MapStart(len) => return Ok(Some(en::Event::MapStart(Some(len)))),
                Node::End => return Ok(Some(en::Event::End)),
                Node::Leaf(value) => return Ok(Some(en::Event::Value(value))),
                Node::Scalar(value) => match value {
                    Scalar::Value(value) => self.pending.push(Node::Leaf(Leaf::Value(value))),
                    Scalar::Ref(value) => self.pending.push(Node::Ref(*value)),
                    Scalar::Op(value) => self.pending.push(Node::Op(value)),
                    Scalar::Map(value) => self.pending.push(Node::Map(value)),
                    Scalar::Tuple(value) => {
                        let len = value.len();
                        self.pending.push(Node::TupleItems(value.into_iter()));
                        return Ok(Some(en::Event::SeqStart(Some(len))));
                    }
                },
                Node::BorrowedScalar(value) => match &value {
                    Scalar::Value(value) => {
                        self.pending.push(Node::Leaf(Leaf::BorrowedValue(value)))
                    }
                    Scalar::Ref(value) => self.pending.push(Node::BorrowedRef(value)),
                    Scalar::Op(value) => self.pending.push(Node::BorrowedOp(value)),
                    Scalar::Map(value) => self.pending.push(Node::BorrowedMap(value)),
                    Scalar::Tuple(value) => {
                        self.pending.push(Node::BorrowedTupleItems(value.iter()));
                        return Ok(Some(en::Event::SeqStart(Some(value.len()))));
                    }
                },
                Node::Map(values) => {
                    let len = values.len();
                    self.pending.push(Node::MapItems(values.into_iter()));
                    return Ok(Some(en::Event::MapStart(Some(len))));
                }
                Node::BorrowedMap(values) => {
                    self.pending
                        .push(Node::BorrowedMapItems(values.into_iter()));
                    return Ok(Some(en::Event::MapStart(Some(values.len()))));
                }
                Node::Form(values) => {
                    let len = values.len();
                    self.pending.push(Node::FormItems(values.into_iter()));
                    return Ok(Some(en::Event::SeqStart(Some(len))));
                }
                Node::BorrowedForm(values) => {
                    self.pending.push(Node::BorrowedFormItems(values.iter()));
                    return Ok(Some(en::Event::SeqStart(Some(values.len()))));
                }
                Node::TupleItems(mut items) => match items.next() {
                    Some(value) => self
                        .pending
                        .extend([Node::TupleItems(items), Node::Scalar(value)]),
                    None => return Ok(Some(en::Event::End)),
                },
                Node::BorrowedTupleItems(mut items) => match items.next() {
                    Some(value) => self
                        .pending
                        .extend([Node::BorrowedTupleItems(items), Node::BorrowedScalar(value)]),
                    None => return Ok(Some(en::Event::End)),
                },
                Node::MapItems(mut items) => match items.next() {
                    Some((id, value)) => {
                        self.pending
                            .extend([Node::MapItems(items), Node::Scalar(value)]);
                        self.text(id);
                    }
                    None => return Ok(Some(en::Event::End)),
                },
                Node::BorrowedMapItems(mut items) => match items.next() {
                    Some((id, value)) => {
                        self.pending
                            .extend([Node::BorrowedMapItems(items), Node::BorrowedScalar(value)]);
                        self.text(id);
                    }
                    None => return Ok(Some(en::Event::End)),
                },
                Node::FormItems(mut items) => match items.next() {
                    Some((id, value)) => {
                        self.pending.extend([
                            Node::FormItems(items),
                            Node::End,
                            Node::Scalar(value),
                        ]);
                        self.text(id);
                        self.pending.push(Node::SeqStart(2));
                    }
                    None => return Ok(Some(en::Event::End)),
                },
                Node::BorrowedFormItems(mut items) => match items.next() {
                    Some((id, value)) => {
                        self.pending.extend([
                            Node::BorrowedFormItems(items),
                            Node::End,
                            Node::BorrowedScalar(value),
                        ]);
                        self.text(id);
                        self.pending.push(Node::SeqStart(2));
                    }
                    None => return Ok(Some(en::Event::End)),
                },
                Node::Ref(value) => match value {
                    TCRef::Op(value) => self.pending.push(Node::Call(value)),
                    TCRef::Id(value) => {
                        self.pending
                            .extend([Node::End, Node::End, Node::SeqStart(0)]);
                        self.tag(value.to_string());
                    }
                    TCRef::Cond(value) => {
                        let crate::Cond {
                            cond,
                            then,
                            or_else,
                        } = *value;
                        self.pending.extend([
                            Node::End,
                            Node::End,
                            Node::Scalar(or_else),
                            Node::Scalar(then),
                            Node::Ref(cond),
                            Node::SeqStart(3),
                        ]);
                        self.tag(crate::TCREF_COND.to_string());
                    }
                    TCRef::After(value) => {
                        let crate::After { when, then } = *value;
                        self.pending.push(Node::End);
                        self.fields([when, then].map(Node::Scalar));
                        self.tag(crate::TCREF_AFTER.to_string());
                    }
                    TCRef::While(value) => {
                        let crate::While {
                            cond,
                            closure,
                            state,
                        } = *value;
                        self.pending.push(Node::End);
                        self.fields([cond, closure, state].map(Node::Scalar));
                        self.tag(crate::TCREF_WHILE.to_string());
                    }
                    TCRef::ForEach(value) => {
                        let crate::ForEach {
                            items,
                            op,
                            item_name,
                        } = *value;
                        self.pending.extend([Node::End, Node::End]);
                        self.text(item_name);
                        self.pending.extend([
                            Node::Scalar(op),
                            Node::Scalar(items),
                            Node::SeqStart(3),
                        ]);
                        self.tag(crate::TCREF_FOR_EACH.to_string());
                    }
                },
                Node::BorrowedRef(value) => match &value {
                    TCRef::Op(value) => self.pending.push(Node::BorrowedCall(value)),
                    TCRef::Id(value) => {
                        self.pending
                            .extend([Node::End, Node::End, Node::SeqStart(0)]);
                        self.tag(value.to_string());
                    }
                    TCRef::Cond(value) => {
                        self.pending.extend([
                            Node::End,
                            Node::End,
                            Node::BorrowedScalar(&value.or_else),
                            Node::BorrowedScalar(&value.then),
                            Node::BorrowedRef(&value.cond),
                            Node::SeqStart(3),
                        ]);
                        self.tag(crate::TCREF_COND.to_string());
                    }
                    TCRef::After(value) => {
                        self.pending.push(Node::End);
                        self.fields([&value.when, &value.then].map(Node::BorrowedScalar));
                        self.tag(crate::TCREF_AFTER.to_string());
                    }
                    TCRef::While(value) => {
                        self.pending.push(Node::End);
                        self.fields(
                            [&value.cond, &value.closure, &value.state].map(Node::BorrowedScalar),
                        );
                        self.tag(crate::TCREF_WHILE.to_string());
                    }
                    TCRef::ForEach(value) => {
                        self.pending.extend([Node::End, Node::End]);
                        self.text(&value.item_name);
                        self.pending.extend([
                            Node::BorrowedScalar(&value.op),
                            Node::BorrowedScalar(&value.items),
                            Node::SeqStart(3),
                        ]);
                        self.tag(crate::TCREF_FOR_EACH.to_string());
                    }
                },
                Node::Call(value) => {
                    self.pending.push(Node::End);
                    match value {
                        OpRef::Get((subject, key)) => {
                            self.fields([key].map(Node::Scalar));
                            self.tag(subject.to_string());
                        }
                        OpRef::Put((subject, key, value)) => {
                            self.fields([key, value].map(Node::Scalar));
                            self.tag(subject.to_string());
                        }
                        OpRef::Post((subject, params)) => {
                            self.pending.push(Node::Map(params));
                            self.tag(subject.to_string());
                        }
                        OpRef::Delete((subject, key)) => {
                            self.pending.extend([Node::End, Node::Scalar(key)]);
                            self.text(subject);
                            self.pending.push(Node::SeqStart(2));
                            self.tag(crate::OPREF_DELETE.to_string());
                        }
                    }
                }
                Node::BorrowedCall(value) => {
                    self.pending.push(Node::End);
                    match value {
                        OpRef::Get((subject, key)) => {
                            self.fields([key].map(Node::BorrowedScalar));
                            self.tag(subject.to_string());
                        }
                        OpRef::Put((subject, key, value)) => {
                            self.fields([key, value].map(Node::BorrowedScalar));
                            self.tag(subject.to_string());
                        }
                        OpRef::Post((subject, params)) => {
                            self.pending.push(Node::BorrowedMap(params));
                            self.tag(subject.to_string());
                        }
                        OpRef::Delete((subject, key)) => {
                            self.pending.extend([Node::End, Node::BorrowedScalar(key)]);
                            self.text(subject);
                            self.pending.push(Node::SeqStart(2));
                            self.tag(crate::OPREF_DELETE.to_string());
                        }
                    }
                }
                Node::Op(value) => {
                    let tag = value.class().path().to_string();
                    self.pending.push(Node::End);
                    match value {
                        OpDef::Get((key, form)) | OpDef::Delete((key, form)) => {
                            self.pending.extend([Node::End, Node::Form(form)]);
                            self.text(key);
                            self.pending.push(Node::SeqStart(2));
                        }
                        OpDef::Put((key, value, form)) => {
                            self.pending.extend([Node::End, Node::Form(form)]);
                            self.text(value);
                            self.text(key);
                            self.pending.push(Node::SeqStart(3));
                        }
                        OpDef::Post(form) => self.pending.push(Node::Form(form)),
                    }
                    self.tag(tag);
                }
                Node::BorrowedOp(value) => {
                    let tag = value.class().path().to_string();
                    self.pending.push(Node::End);
                    match value {
                        OpDef::Get((key, form)) | OpDef::Delete((key, form)) => {
                            self.pending.extend([Node::End, Node::BorrowedForm(form)]);
                            self.text(key);
                            self.pending.push(Node::SeqStart(2));
                        }
                        OpDef::Put((key, value, form)) => {
                            self.pending.extend([Node::End, Node::BorrowedForm(form)]);
                            self.text(value);
                            self.text(key);
                            self.pending.push(Node::SeqStart(3));
                        }
                        OpDef::Post(form) => self.pending.push(Node::BorrowedForm(form)),
                    }
                    self.tag(tag);
                }
            }
        }
    }
}

macro_rules! encode_root {
    ($ty:ty, $owned:ident, $borrowed:ident) => {
        impl<'en> IntoStream<'en> for $ty {
            fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
                let mut events = Events::new::<E::Error>(Node::$owned(self))?;
                encoder.encode_events(stream::iter(std::iter::from_fn(move || {
                    match events.next_event() {
                        Ok(event) => event.map(Ok),
                        Err(error) => {
                            events.pending.clear();
                            Some(Err(error))
                        }
                    }
                })))
            }
        }

        impl<'en> ToStream<'en> for $ty {
            fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
                let mut events = Events::new::<E::Error>(Node::$borrowed(self))?;
                encoder.encode_events(stream::iter(std::iter::from_fn(move || {
                    match events.next_event() {
                        Ok(event) => event.map(Ok),
                        Err(error) => {
                            events.pending.clear();
                            Some(Err(error))
                        }
                    }
                })))
            }
        }
    };
}

encode_root!(Scalar, Scalar, BorrowedScalar);
encode_root!(TCRef, Ref, BorrowedRef);
encode_root!(OpDef, Op, BorrowedOp);
encode_root!(OpRef, Call, BorrowedCall);

#[cfg(test)]
#[path = "../../tests/unit/codec/encode.rs"]
mod tests;
