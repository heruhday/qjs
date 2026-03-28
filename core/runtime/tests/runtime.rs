use std::cell::RefCell;
use std::rc::Rc;

use runtime::{Context, Runtime};

#[test]
fn creates_context_and_sets_current() {
    let rt = Rc::new(RefCell::new(Runtime::new()));
    let ctx = Context::new(rt);

    let current_atoms = Context::with_current(|current| current.atom_count());
    assert_eq!(current_atoms, 0);
    assert_eq!(ctx.object_count(), 2);
}

#[test]
fn interns_strings_and_allocates_objects() {
    let rt = Rc::new(RefCell::new(Runtime::new()));
    let ctx = Context::new(rt);

    let atom = ctx.intern("hello");
    let object = ctx.new_object();
    let string = ctx.new_string("hello");

    assert_eq!(ctx.resolve(atom), "hello");
    assert!(object.borrow().prototype.is_some());
    assert_eq!(string.borrow().text(&ctx.rt.borrow().atoms), "hello");
    assert_eq!(ctx.atom_count(), 1);
    assert_eq!(ctx.object_count(), 4);
}
