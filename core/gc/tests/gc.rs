use std::ptr;

use gc::{CollectStats, GC, Gc, HeapKind, HeapTyped, ObjType};

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
fn allocates_and_tracks_objects() {
    let mut gc = GC::new();
    let value = Gc::new(&mut gc, TestObject(7));

    assert_eq!(gc.object_count(), 1);
    assert!(gc.contains_ptr(value.header_ptr()));
    assert!(!gc.contains_ptr(ptr::null()));
}

#[test]
fn supports_shared_borrow_and_mutation() {
    let mut gc = GC::new();
    let value = Gc::new(&mut gc, TestObject(7));
    let clone = value.clone();

    clone.borrow_mut().0 = 9;

    assert!(value.ptr_eq(&clone));
    assert_eq!(value.borrow().0, 9);
}

#[test]
fn assigns_string_object_headers() {
    let mut gc = GC::new();
    let value = Gc::new(&mut gc, TestString("hello"));

    assert_eq!(value.borrow().0, "hello");
    assert_eq!(value.header().obj_type, ObjType::String);
    assert_eq!(value.header().kind, HeapKind::String);
}

#[test]
fn gc_box_is_16_byte_aligned() {
    let mut gc = GC::new();
    let value = Gc::new(&mut gc, TestObject(1));

    assert_eq!((value.as_ptr() as usize) % 16, 0);
}

#[test]
fn collect_reports_noop_stats_for_now() {
    let mut gc = GC::new();
    let _ = Gc::new(&mut gc, TestObject(1));
    let _ = Gc::new(&mut gc, TestObject(2));
    let mut atoms = ();
    let stats = gc.collect::<(), _>(&[], &mut atoms);

    assert_eq!(
        stats,
        CollectStats {
            before: 2,
            after: 2,
            collected: 0,
        }
    );
}
