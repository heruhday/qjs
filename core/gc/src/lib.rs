use std::{
    any::Any,
    cell::{Ref, RefCell, RefMut},
    ptr,
    rc::Rc,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollectStats {
    pub before: usize,
    pub after: usize,
    pub collected: usize,
}

#[allow(dead_code)]
struct TrackedAllocation {
    header_ptr: *const GCHeader,
    _owner: Rc<dyn Any>,
}

/// Standalone GC allocator state extracted from the VM crate.
///
/// This crate currently provides allocation, tagging, and tracking primitives.
/// The VM-specific mark/sweep traversal from the original source is intentionally
/// left out until the runtime types exist in this workspace.
#[allow(clippy::upper_case_acronyms)]
#[derive(Default)]
pub struct GC {
    allocated: usize,
    objects: Vec<TrackedAllocation>,
}

impl GC {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn object_count(&self) -> usize {
        self.allocated
    }

    pub fn contains_ptr(&self, ptr: *const GCHeader) -> bool {
        self.objects
            .iter()
            .any(|entry| ptr::eq(entry.header_ptr, ptr))
    }

    pub fn collect<R, A>(&mut self, _roots: &[R], _atoms: &mut A) -> CollectStats {
        CollectStats {
            before: self.allocated,
            after: self.allocated,
            collected: 0,
        }
    }
}

impl Drop for GC {
    fn drop(&mut self) {
        self.objects.clear();
    }
}

pub trait Trace {}

impl<T> Trace for T {}

pub trait AtomTrace {}

impl<T> AtomTrace for T {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapKind {
    Object,
    Array,
    BoolArray,
    Uint8Array,
    Int32Array,
    Float64Array,
    StringArray,
    String,
    Symbol,
    Function,
    Closure,
    NativeFunction,
    NativeClosure,
    Class,
    Module,
    Instance,
}

impl HeapKind {
    pub fn type_name(self) -> &'static str {
        match self {
            HeapKind::Object => "object",
            HeapKind::Array => "array",
            HeapKind::BoolArray => "bool array",
            HeapKind::Uint8Array => "uint8 array",
            HeapKind::Int32Array => "int32 array",
            HeapKind::Float64Array => "float64 array",
            HeapKind::StringArray => "string array",
            HeapKind::String => "string object",
            HeapKind::Symbol => "symbol object",
            HeapKind::Function => "function",
            HeapKind::Closure => "closure",
            HeapKind::NativeFunction => "native function",
            HeapKind::NativeClosure => "native closure",
            HeapKind::Class => "class",
            HeapKind::Module => "module",
            HeapKind::Instance => "instance",
        }
    }
}

pub trait HeapTyped {
    const KIND: HeapKind;
}

#[repr(C, align(16))]
#[derive(Debug)]
pub struct GcBox<T> {
    pub header: GCHeader,
    pub value: RefCell<T>,
}

#[derive(Debug)]
pub struct Gc<T> {
    pub(crate) inner: Rc<GcBox<T>>,
}

impl<T: HeapTyped + 'static> Gc<T> {
    pub fn new(gc: &mut GC, value: T) -> Self {
        gc.allocated += 1;

        let obj_type = match T::KIND {
            HeapKind::String => ObjType::String,
            _ => ObjType::Object,
        };

        let inner = Rc::new(GcBox {
            header: GCHeader::with_kind(obj_type, T::KIND),
            value: RefCell::new(value),
        });
        let header_ptr = ptr::addr_of!(inner.header);

        gc.objects.push(TrackedAllocation {
            header_ptr,
            _owner: inner.clone() as Rc<dyn Any>,
        });

        Self { inner }
    }
}

impl<T> Gc<T> {
    /// Reconstructs a shared `Gc<T>` from a raw allocation pointer.
    ///
    /// # Safety
    /// The pointer must come from a live allocation produced by this crate and
    /// still be backed by at least one strong `Rc` reference.
    pub unsafe fn clone_from_raw(ptr: *const GcBox<T>) -> Self {
        unsafe {
            Rc::increment_strong_count(ptr);
            Self {
                inner: Rc::from_raw(ptr),
            }
        }
    }

    pub fn borrow(&self) -> Ref<'_, T> {
        self.inner.value.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.inner.value.borrow_mut()
    }

    pub fn as_ptr(&self) -> *const GcBox<T> {
        Rc::as_ptr(&self.inner)
    }

    pub fn header_ptr(&self) -> *const GCHeader {
        ptr::addr_of!(self.inner.header)
    }

    pub fn header(&self) -> &GCHeader {
        &self.inner.header
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T> Clone for Gc<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> PartialEq for Gc<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T> Eq for Gc<T> {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjType {
    Object,
    String,
    Shape,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GCHeader {
    pub marked: bool,
    pub obj_type: ObjType,
    pub kind: HeapKind,
}

impl GCHeader {
    pub fn new(obj_type: ObjType) -> Self {
        let kind = match obj_type {
            ObjType::Object => HeapKind::Object,
            ObjType::String => HeapKind::String,
            ObjType::Shape => HeapKind::Object,
        };

        Self {
            marked: false,
            obj_type,
            kind,
        }
    }

    pub fn with_kind(obj_type: ObjType, kind: HeapKind) -> Self {
        Self {
            marked: false,
            obj_type,
            kind,
        }
    }
}
