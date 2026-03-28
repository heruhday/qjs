use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use atoms::{Atom, AtomTable, Shape};
use gc::{GCHeader, HeapTyped, ObjType};
use value::{HeapKind, JSValue, make_int32, make_undefined};

pub trait RuntimeContext {}

#[derive(Debug, Clone)]
pub struct QObject {
    pub shape: Rc<Shape>,
    pub values: Vec<JSValue>,
    pub prototype: Option<JSValue>,
    pub is_html_dda: bool,
}

impl QObject {
    pub fn new(shape: Rc<Shape>) -> Self {
        Self {
            shape,
            values: vec![],
            prototype: None,
            is_html_dda: false,
        }
    }

    pub fn get(&self, atom: Atom) -> Option<JSValue> {
        self.shape
            .props
            .get(&atom)
            .and_then(|&index| self.values.get(index))
            .copied()
    }

    pub fn set(&mut self, atom: Atom, value: JSValue) {
        let next_shape = if self.shape.props.contains_key(&atom) {
            self.shape.clone()
        } else {
            self.shape.transition(atom)
        };
        let index = *next_shape
            .props
            .get(&atom)
            .expect("transition must install atom");
        self.shape = next_shape;
        if index >= self.values.len() {
            self.values.resize(index + 1, make_undefined());
        }
        self.values[index] = value;
    }

    pub fn trace_atoms(&self, atoms: &mut AtomTable) {
        self.shape.trace_atoms(atoms);
    }
}

#[derive(Debug, Clone)]
pub struct QArray {
    pub elements: Vec<JSValue>,
    pub sparse: Option<BTreeMap<usize, JSValue>>,
    pub length: u64,
    pub object: QObject,
    pub shared_buffer: Option<Vec<i32>>,
    pub shared_offset: usize,
    pub shared_length: usize,
}

impl QArray {
    pub fn new(shape: Rc<Shape>) -> Self {
        Self {
            elements: vec![],
            sparse: None,
            length: 0,
            object: QObject::new(shape),
            shared_buffer: None,
            shared_offset: 0,
            shared_length: 0,
        }
    }

    pub fn push(&mut self, value: JSValue) {
        self.elements.push(value);
        self.length = self.length.max(self.elements.len() as u64);
    }

    pub fn get(&self, index: usize) -> Option<JSValue> {
        if let Some(shared) = &self.shared_buffer
            && index < self.shared_length
        {
            let offset = self.shared_offset.saturating_add(index);
            return Some(
                shared
                    .get(offset)
                    .copied()
                    .map(make_int32)
                    .unwrap_or_else(make_undefined),
            );
        }

        self.elements.get(index).copied().or_else(|| {
            self.sparse
                .as_ref()
                .and_then(|sparse| sparse.get(&index).copied())
        })
    }
}

macro_rules! define_plain_array {
    ($name:ident, $item:ty) => {
        #[derive(Debug, Clone)]
        pub struct $name {
            pub elements: Vec<$item>,
            pub object: QObject,
        }

        impl $name {
            pub fn new(shape: Rc<Shape>) -> Self {
                Self {
                    elements: vec![],
                    object: QObject::new(shape),
                }
            }

            pub fn push(&mut self, value: $item) {
                self.elements.push(value);
            }

            pub fn get(&self, index: usize) -> Option<$item> {
                self.elements.get(index).copied()
            }
        }
    };
}

define_plain_array!(QBoolArray, bool);
define_plain_array!(QUint8Array, u8);
define_plain_array!(QInt32Array, i32);
define_plain_array!(QFloat64Array, f64);

#[derive(Debug, Clone)]
pub struct QStringArray {
    pub elements: Vec<JSValue>,
    pub object: QObject,
}

impl QStringArray {
    pub fn new(shape: Rc<Shape>) -> Self {
        Self {
            elements: vec![],
            object: QObject::new(shape),
        }
    }

    pub fn push(&mut self, value: JSValue) {
        self.elements.push(value);
    }

    pub fn get(&self, index: usize) -> Option<JSValue> {
        self.elements.get(index).copied()
    }
}

#[repr(C, align(16))]
#[derive(Debug, Clone)]
pub struct QString {
    pub header: GCHeader,
    pub atom: Atom,
}

impl QString {
    pub fn new(atom: Atom) -> Self {
        Self {
            header: GCHeader::new(ObjType::String),
            atom,
        }
    }

    pub fn text<'a>(&self, atoms: &'a AtomTable) -> &'a str {
        atoms.resolve(self.atom)
    }
}

#[derive(Debug, Clone)]
pub struct QSymbol {
    pub id: u64,
    pub description: Option<Atom>,
}

pub type NativeFunctionCallback = fn(&dyn RuntimeContext, JSValue, &[JSValue]) -> JSValue;

#[derive(Debug, Clone)]
pub struct QFunction {
    pub name: Option<Atom>,
    pub params: Vec<Atom>,
    pub body: Vec<JSValue>,
    pub prototype: Option<JSValue>,
    pub descriptor: JSValue,
    pub upvalues: Vec<JSValue>,
}

#[derive(Debug, Clone)]
pub struct QClosure {
    pub function: JSValue,
    pub captures: Vec<JSValue>,
}

#[derive(Debug, Clone)]
pub struct QNativeFunction {
    pub name: Option<Atom>,
    pub callback: NativeFunctionCallback,
}

#[derive(Debug, Clone)]
pub struct QNativeClosure {
    pub name: Option<Atom>,
    pub callback: NativeFunctionCallback,
    pub captures: Vec<JSValue>,
}

#[derive(Debug, Clone)]
pub struct QClass {
    pub name: Option<Atom>,
    pub prototype: Option<JSValue>,
    pub constructor: Option<JSValue>,
    pub static_props: HashMap<Atom, JSValue>,
    pub base: JSValue,
}

#[derive(Debug, Clone)]
pub struct QModule {
    pub name: Option<Atom>,
    pub exports: HashMap<Atom, JSValue>,
}

#[derive(Debug, Clone)]
pub struct QInstance {
    pub class: JSValue,
    pub object: QObject,
}

impl HeapTyped for QObject {
    const KIND: HeapKind = HeapKind::Object;
}

impl HeapTyped for QArray {
    const KIND: HeapKind = HeapKind::Array;
}

impl HeapTyped for QBoolArray {
    const KIND: HeapKind = HeapKind::BoolArray;
}

impl HeapTyped for QUint8Array {
    const KIND: HeapKind = HeapKind::Uint8Array;
}

impl HeapTyped for QInt32Array {
    const KIND: HeapKind = HeapKind::Int32Array;
}

impl HeapTyped for QFloat64Array {
    const KIND: HeapKind = HeapKind::Float64Array;
}

impl HeapTyped for QStringArray {
    const KIND: HeapKind = HeapKind::StringArray;
}

impl HeapTyped for QString {
    const KIND: HeapKind = HeapKind::String;
}

impl HeapTyped for QSymbol {
    const KIND: HeapKind = HeapKind::Symbol;
}

impl HeapTyped for QFunction {
    const KIND: HeapKind = HeapKind::Function;
}

impl HeapTyped for QClosure {
    const KIND: HeapKind = HeapKind::Closure;
}

impl HeapTyped for QNativeFunction {
    const KIND: HeapKind = HeapKind::NativeFunction;
}

impl HeapTyped for QNativeClosure {
    const KIND: HeapKind = HeapKind::NativeClosure;
}

impl HeapTyped for QClass {
    const KIND: HeapKind = HeapKind::Class;
}

impl HeapTyped for QModule {
    const KIND: HeapKind = HeapKind::Module;
}

impl HeapTyped for QInstance {
    const KIND: HeapKind = HeapKind::Instance;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_set_adds_value() {
        let shape = Rc::new(Shape::new());
        let mut object = QObject::new(shape);
        let atom = Atom(1);

        object.set(atom, JSValue::i32(7));

        assert_eq!(object.get(atom), Some(JSValue::i32(7)));
    }
}
