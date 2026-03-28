use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread_local;

use crate::gc::{CollectStats, GC, Gc};
use crate::js_value::{JSValue as Value, make_undefined};
use atoms::{Atom, AtomTable, Shape};
use heap::{
    NativeFunctionCallback, QArray, QBoolArray, QClass, QClosure, QFloat64Array, QFunction,
    QInstance, QInt32Array, QModule, QNativeClosure, QNativeFunction, QObject, QString,
    QStringArray, QSymbol, QUint8Array, RuntimeContext,
};

thread_local! {
    pub static CURRENT_JS_CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

fn gc_handle_value<T: crate::gc::Trace + crate::gc::AtomTrace + crate::gc::HeapTyped + 'static>(
    handle: &Gc<T>,
) -> Value {
    Value::from(handle.clone())
}

pub trait IntoPropertyKey {
    fn into_property_key(self, ctx: &Context) -> Atom;
}

impl IntoPropertyKey for Atom {
    fn into_property_key(self, _ctx: &Context) -> Atom {
        self
    }
}

impl IntoPropertyKey for &Atom {
    fn into_property_key(self, _ctx: &Context) -> Atom {
        *self
    }
}

impl IntoPropertyKey for &str {
    fn into_property_key(self, ctx: &Context) -> Atom {
        ctx.intern(self)
    }
}

impl IntoPropertyKey for String {
    fn into_property_key(self, ctx: &Context) -> Atom {
        ctx.intern(&self)
    }
}

impl IntoPropertyKey for &String {
    fn into_property_key(self, ctx: &Context) -> Atom {
        ctx.intern(self)
    }
}

pub struct Runtime {
    pub id: u64,
    pub gc: GC,
    pub atoms: AtomTable,
    pub next_symbol_id: u64,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            id: NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
            gc: GC::new(),
            atoms: AtomTable::new(),
            next_symbol_id: 0,
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Context {
    pub rt: Rc<RefCell<Runtime>>,
    pub empty_shape: Rc<Shape>,
    pub object_proto: Gc<QObject>,
    pub array_proto: Gc<QObject>,
    pub globals: Rc<RefCell<HashMap<Atom, Value>>>,
    pub current_strict: Rc<Cell<bool>>,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        Self {
            rt: self.rt.clone(),
            empty_shape: self.empty_shape.clone(),
            object_proto: self.object_proto.clone(),
            array_proto: self.array_proto.clone(),
            globals: self.globals.clone(),
            current_strict: self.current_strict.clone(),
        }
    }
}

impl RuntimeContext for Context {}

impl Context {
    pub fn new(rt: Rc<RefCell<Runtime>>) -> Self {
        let empty_shape = Rc::new(Shape::new());

        let object_proto = {
            let mut runtime = rt.borrow_mut();
            Gc::new(&mut runtime.gc, QObject::new(empty_shape.clone()))
        };

        let array_proto = {
            let mut runtime = rt.borrow_mut();
            let mut obj = QObject::new(empty_shape.clone());
            obj.prototype = Some(gc_handle_value(&object_proto));
            Gc::new(&mut runtime.gc, obj)
        };

        let globals = Rc::new(RefCell::new(HashMap::new()));
        let current_strict = Rc::new(Cell::new(false));

        let ctx = Self {
            rt,
            empty_shape,
            object_proto,
            array_proto,
            globals,
            current_strict,
        };

        CURRENT_JS_CONTEXT.with(|current| {
            *current.borrow_mut() = Some(ctx.clone());
        });

        ctx
    }

    #[inline]
    pub fn with_current<R>(f: impl FnOnce(&Context) -> R) -> R {
        CURRENT_JS_CONTEXT.with(|current| {
            let ctx_ref = current.borrow();
            let ctx = ctx_ref.as_ref().expect(
                "value access requires an active Context; create Context first or pass one explicitly",
            );
            f(ctx)
        })
    }

    #[inline]
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub(crate) fn with_current_opt<R>(f: impl FnOnce(Option<&Context>) -> R) -> R {
        CURRENT_JS_CONTEXT.with(|current| f(current.borrow().as_ref()))
    }

    #[inline]
    pub fn current() -> Self {
        Self::with_current(Clone::clone)
    }

    #[inline]
    pub fn intern(&self, s: &str) -> Atom {
        self.rt.borrow_mut().atoms.intern(s)
    }

    #[inline]
    pub fn resolve(&self, atom: Atom) -> String {
        self.rt.borrow().atoms.resolve(atom).to_string()
    }

    #[inline]
    pub fn with_resolved<R>(&self, atom: Atom, f: impl FnOnce(&str) -> R) -> R {
        let runtime = self.rt.borrow();
        f(runtime.atoms.resolve(atom))
    }

    pub fn new_object(&self) -> Gc<QObject> {
        let mut runtime = self.rt.borrow_mut();

        Gc::new(
            &mut runtime.gc,
            QObject {
                shape: self.empty_shape.clone(),
                values: vec![],
                prototype: Some(gc_handle_value(&self.object_proto)),
                is_html_dda: false,
            },
        )
    }

    pub fn new_array(&self) -> Gc<QArray> {
        let mut runtime = self.rt.borrow_mut();
        let mut array = QArray::new(self.empty_shape.clone());
        array.object.prototype = Some(gc_handle_value(&self.array_proto));
        Gc::new(&mut runtime.gc, array)
    }

    pub fn new_bool_array(&self) -> Gc<QBoolArray> {
        let mut runtime = self.rt.borrow_mut();
        let mut array = QBoolArray::new(self.empty_shape.clone());
        array.object.prototype = Some(gc_handle_value(&self.object_proto));
        Gc::new(&mut runtime.gc, array)
    }

    pub fn new_uint8_array(&self) -> Gc<QUint8Array> {
        let mut runtime = self.rt.borrow_mut();
        let mut array = QUint8Array::new(self.empty_shape.clone());
        array.object.prototype = Some(gc_handle_value(&self.object_proto));
        Gc::new(&mut runtime.gc, array)
    }

    pub fn new_int32_array(&self) -> Gc<QInt32Array> {
        let mut runtime = self.rt.borrow_mut();
        let mut array = QInt32Array::new(self.empty_shape.clone());
        array.object.prototype = Some(gc_handle_value(&self.object_proto));
        Gc::new(&mut runtime.gc, array)
    }

    pub fn new_float64_array(&self) -> Gc<QFloat64Array> {
        let mut runtime = self.rt.borrow_mut();
        let mut array = QFloat64Array::new(self.empty_shape.clone());
        array.object.prototype = Some(gc_handle_value(&self.object_proto));
        Gc::new(&mut runtime.gc, array)
    }

    pub fn new_string_array(&self) -> Gc<QStringArray> {
        let mut runtime = self.rt.borrow_mut();
        let mut array = QStringArray::new(self.empty_shape.clone());
        array.object.prototype = Some(gc_handle_value(&self.object_proto));
        Gc::new(&mut runtime.gc, array)
    }

    pub fn new_string(&self, s: &str) -> Gc<QString> {
        let atom = self.intern(s);
        let mut runtime = self.rt.borrow_mut();
        Gc::new(&mut runtime.gc, QString::new(atom))
    }

    pub fn new_symbol(&self, description: Option<&str>) -> Gc<QSymbol> {
        let desc = description.map(|text| self.intern(text));
        let mut runtime = self.rt.borrow_mut();
        let id = runtime.next_symbol_id;
        runtime.next_symbol_id += 1;

        Gc::new(
            &mut runtime.gc,
            QSymbol {
                id,
                description: desc,
            },
        )
    }

    pub fn new_function(
        &self,
        name: Option<&str>,
        params: &[&str],
        body: Vec<Value>,
    ) -> Gc<QFunction> {
        let name = name.map(|text| self.intern(text));
        let params = params.iter().map(|param| self.intern(param)).collect();
        let prototype = self.new_object();
        let mut runtime = self.rt.borrow_mut();

        Gc::new(
            &mut runtime.gc,
            QFunction {
                name,
                params,
                body,
                prototype: Some(gc_handle_value(&prototype)),
                descriptor: make_undefined(),
                upvalues: Vec::new(),
            },
        )
    }

    pub fn new_closure(&self, function: Gc<QFunction>, captures: Vec<Value>) -> Gc<QClosure> {
        let mut runtime = self.rt.borrow_mut();
        Gc::new(
            &mut runtime.gc,
            QClosure {
                function: gc_handle_value(&function),
                captures,
            },
        )
    }

    pub fn new_native_function(
        &self,
        name: Option<&str>,
        callback: NativeFunctionCallback,
    ) -> Gc<QNativeFunction> {
        let name = name.map(|text| self.intern(text));
        let mut runtime = self.rt.borrow_mut();

        Gc::new(&mut runtime.gc, QNativeFunction { name, callback })
    }

    pub fn new_native_closure(
        &self,
        name: Option<&str>,
        callback: NativeFunctionCallback,
        captures: Vec<Value>,
    ) -> Gc<QNativeClosure> {
        let name = name.map(|text| self.intern(text));
        let mut runtime = self.rt.borrow_mut();

        Gc::new(
            &mut runtime.gc,
            QNativeClosure {
                name,
                callback,
                captures,
            },
        )
    }

    pub fn new_class(&self, name: &str, constructor: Option<Value>) -> Gc<QClass> {
        let name = self.intern(name);
        let prototype = self.new_object();
        let mut runtime = self.rt.borrow_mut();

        Gc::new(
            &mut runtime.gc,
            QClass {
                name: Some(name),
                prototype: Some(gc_handle_value(&prototype)),
                constructor,
                static_props: HashMap::new(),
                base: make_undefined(),
            },
        )
    }

    pub fn new_module(&self, name: &str) -> Gc<QModule> {
        let name = self.intern(name);
        let mut runtime = self.rt.borrow_mut();

        Gc::new(
            &mut runtime.gc,
            QModule {
                name: Some(name),
                exports: HashMap::new(),
            },
        )
    }

    pub fn new_instance(&self, class: Gc<QClass>) -> Gc<QInstance> {
        let prototype = class.borrow().prototype;
        let mut runtime = self.rt.borrow_mut();

        Gc::new(
            &mut runtime.gc,
            QInstance {
                class: gc_handle_value(&class),
                object: QObject {
                    shape: self.empty_shape.clone(),
                    values: vec![],
                    prototype,
                    is_html_dda: false,
                },
            },
        )
    }

    pub fn collect(&self, roots: &[Value]) -> CollectStats {
        let mut runtime = self.rt.borrow_mut();
        let Runtime { gc, atoms, .. } = &mut *runtime;
        gc.collect(roots, atoms)
    }

    #[inline]
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub(crate) fn is_valid_heap_ptr(&self, ptr: *const crate::gc::GCHeader) -> bool {
        match self.rt.try_borrow() {
            Ok(runtime) => runtime.gc.contains_ptr(ptr),
            Err(_) => true,
        }
    }

    pub fn object_count(&self) -> usize {
        self.rt.borrow().gc.object_count()
    }

    pub fn atom_count(&self) -> usize {
        self.rt.borrow().atoms.count()
    }

    #[inline]
    pub fn set_current_strict(&self, strict: bool) {
        self.current_strict.set(strict);
    }

    #[inline]
    pub fn current_strict(&self) -> bool {
        self.current_strict.get()
    }
}
