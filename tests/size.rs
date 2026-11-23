use std::mem::size_of;

use get_size::GetSize;

use tc_ir::{After, Id, OpDef, Scalar, TCRef};
use tc_value::Value;

#[test]
fn retained_size_counts_spare_capacity_and_boxed_syntax() {
    let mut text = String::with_capacity(64);
    text.push_str("value");
    let text_capacity = text.capacity();
    let mut tuple = Vec::with_capacity(7);
    tuple.push(Scalar::Value(Value::String(text)));
    let expected = tuple.capacity() * size_of::<Scalar>() + text_capacity;
    let value = Scalar::Tuple(tuple);
    assert_eq!(value.get_heap_size(), expected);

    let id: Id = "capture".parse().unwrap();
    let id_heap = id.get_heap_size();
    let mut form = Vec::with_capacity(9);
    form.push((id, value));
    let expected = form.capacity() * size_of::<(Id, Scalar)>() + id_heap + expected;
    let op = Scalar::Op(OpDef::Post(form));
    assert_eq!(op.get_heap_size(), expected);

    let wrapped = Scalar::Ref(Box::new(TCRef::After(Box::new(After::new(
        Scalar::Value(Value::None),
        op,
    )))));
    assert_eq!(
        wrapped.get_size(),
        size_of::<Scalar>() + size_of::<TCRef>() + size_of::<After>() + expected
    );
}

#[test]
fn retained_size_at_supported_nesting() {
    let depth = tc_ir::MAX_SCALAR_DEPTH;
    let mut value = Scalar::Value(Value::None);
    for _ in 0..depth {
        value = Scalar::Tuple(vec![value]);
    }
    let actual = value.get_size();
    drop(value);
    assert_eq!(actual, (depth + 1) * size_of::<Scalar>());
}
