use gc::{GC, Gc, HeapKind, HeapTyped};
use value::{
    Atom, JSValue, Null, Undefined, Value, bool_from_value, is_number, is_object, is_string,
    make_bool, make_heap, make_int32, make_null, make_number, make_undefined, string_from_value,
    to_f64, to_i32,
};

#[derive(Debug, PartialEq, Eq)]
struct TestObject(u32);

impl HeapTyped for TestObject {
    const KIND: HeapKind = HeapKind::Object;
}

#[derive(Debug, PartialEq, Eq)]
struct TestString(&'static str);

impl HeapTyped for TestString {
    const KIND: HeapKind = HeapKind::String;
}

#[test]
fn primitive_tags_round_trip() {
    let int = make_int32(42);
    let float = make_number(3.5);
    let boolean = make_bool(true);
    let null = make_null();
    let undefined = make_undefined();
    let atom = Value::from(Atom(7));

    assert_eq!(to_i32(int), Some(42));
    assert_eq!(to_f64(float), Some(3.5));
    assert_eq!(bool_from_value(boolean), Some(true));
    assert!(Null::try_from(null).is_ok());
    assert!(Undefined::try_from(undefined).is_ok());
    assert_eq!(Atom::try_from(atom), Ok(Atom(7)));
    assert!(is_number(int));
    assert!(is_string(atom));
}

#[test]
fn heap_values_round_trip_through_gc() {
    let mut gc = GC::new();
    let object = Gc::new(&mut gc, TestObject(9));
    let value = make_heap(object.header_ptr());
    let recovered: Gc<TestObject> = value.try_into().expect("typed GC value");

    assert!(is_object(value));
    assert_eq!(value.heap_kind(), Some(HeapKind::Object));
    assert_eq!(recovered.borrow().0, 9);
}

#[test]
fn string_heap_values_are_detected() {
    let mut gc = GC::new();
    let string = Gc::new(&mut gc, TestString("hello"));
    let value: JSValue = make_heap(string.header_ptr());

    assert!(is_string(value));
    assert!(string_from_value(value).is_some());
    assert_eq!(value.heap_kind(), Some(HeapKind::String));
}

#[test]
fn numeric_try_from_uses_ecma_number_shape() {
    assert_eq!(u8::try_from(Value::i32(255)), Ok(255));
    assert_eq!(i16::try_from(Value::f64(12.0)), Ok(12));
    assert!(u8::try_from(Value::f64(-1.0)).is_err());
}

#[test]
fn debug_representation_is_informative() {
    assert_eq!(format!("{:?}", Value::i32(1)), "Int(1)");
    assert_eq!(format!("{:?}", Value::bool(false)), "Bool(false)");
    assert_eq!(format!("{:?}", Value::null()), "Null");
}
