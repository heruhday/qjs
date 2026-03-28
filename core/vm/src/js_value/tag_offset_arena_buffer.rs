#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::mem::{MaybeUninit, align_of, size_of};

use crate::atoms::Atom;
use crate::gc::Gc;
use crate::heap::{QArray, QObject, QString};
use crate::runtime::Context;

use super::{HeapKind, Value};

#[repr(transparent)]
#[derive(Clone, Copy)]
struct Tag(usize);

const TAG_NULL: Tag = Tag(0);
const TAG_BOOL: Tag = Tag(1);
const TAG_I32: Tag = Tag(2);
const TAG_F64: Tag = Tag(3);
const TAG_STRING: Tag = Tag(4);
const TAG_ARRAY: Tag = Tag(5);
const TAG_OBJECT: Tag = Tag(6);
const TAG_FUNCTION: Tag = Tag(7);
const TAG_CLOSURE: Tag = Tag(8);
const TAG_ITERATOR: Tag = Tag(9);
const TAG_CLASS: Tag = Tag(10);
const TAG_MODULE: Tag = Tag(11);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
struct StringIndex(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
struct OffsetArray(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
struct OffsetObject(usize);

#[derive(Clone, Copy)]
#[repr(C)]
union ValueRepr {
    null: usize,
    bool_: bool,
    i32_: i32,
    f64_: f64,
    string: StringIndex,
    array: OffsetArray,
    object: OffsetObject,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct SerializedValue {
    tag: Tag,
    repr: ValueRepr,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct SerializedDocument {
    string_table_offset: usize,
    root: SerializedValue,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct StringData {
    len: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct StringTable {
    len: usize,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct ArrayData {
    len: usize,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct ObjectEntry {
    key: StringIndex,
    value: SerializedValue,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct ObjectData {
    len: usize,
}

#[derive(Debug)]
pub enum ValueError {
    ArenaFull,
    InvalidTag(usize),
    InvalidValueShape(&'static str),
    InvalidUtf8(std::str::Utf8Error),
    StringTooLong(usize),
    Json(serde_json::Error),
    NonFiniteFloat(f64),
    MisalignedOffset {
        offset: usize,
        align: usize,
    },
    OutOfBounds {
        offset: usize,
        len: usize,
        arena_len: usize,
    },
    OffsetOverflow(u64),
    SizeOverflow,
}

impl std::fmt::Display for ValueError {
    #[inline(always)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueError::ArenaFull => f.write_str("arena buffer full"),
            ValueError::InvalidTag(tag) => write!(f, "invalid tag {tag}"),
            ValueError::InvalidValueShape(msg) => f.write_str(msg),
            ValueError::InvalidUtf8(err) => err.fmt(f),
            ValueError::StringTooLong(len) => write!(f, "string too long: {len}"),
            ValueError::Json(err) => err.fmt(f),
            ValueError::NonFiniteFloat(value) => write!(f, "non-finite float {value}"),
            ValueError::MisalignedOffset { offset, align } => {
                write!(f, "misaligned offset {offset} for align {align}")
            }
            ValueError::OutOfBounds {
                offset,
                len,
                arena_len,
            } => write!(
                f,
                "out of bounds read offset={offset} len={len} arena_len={arena_len}"
            ),
            ValueError::OffsetOverflow(value) => write!(f, "offset overflow {value}"),
            ValueError::SizeOverflow => f.write_str("size overflow"),
        }
    }
}

impl std::error::Error for ValueError {}

impl From<std::str::Utf8Error> for ValueError {
    #[inline(always)]
    fn from(value: std::str::Utf8Error) -> Self {
        Self::InvalidUtf8(value)
    }
}

pub(super) fn to_arena_buffer(ctx: &Context, value: Value) -> Result<Vec<u8>, ValueError> {
    let mut collector = StringCollector::new(ctx);
    collector.collect(value)?;

    let string_table_bytes = encode_string_table(ctx, &collector.strings)?;
    let document_size = size_of::<SerializedDocument>();
    let string_table_offset = align_up(document_size, align_of::<StringTable>());
    let heap_base = align_up(
        checked_add(string_table_offset, string_table_bytes.len())?,
        heap_align(),
    );

    let mut encoder = ArenaEncoder::new(
        collector.string_index,
        heap_base,
        collector.estimated_heap_bytes,
        collector.array_count,
        collector.object_count,
        collector.has_shared_references,
    );
    let root = encoder.encode_value(value)?;

    let total_len = checked_add(heap_base, encoder.heap.len())?;
    let mut out = Vec::with_capacity(total_len);

    append_padding(&mut out, string_table_offset);
    write_struct_at(
        &mut out,
        0,
        &SerializedDocument {
            string_table_offset,
            root,
        },
    )?;

    out.extend_from_slice(&string_table_bytes);
    if out.len() < heap_base {
        out.resize(heap_base, 0);
    }
    out.extend_from_slice(&encoder.heap);

    Ok(out)
}

pub(super) fn from_arena_buffer(ctx: &Context, bytes: &[u8]) -> Result<Value, ValueError> {
    let document = read_struct::<SerializedDocument>(bytes, 0)?;
    let strings = parse_string_table(ctx, bytes, document.string_table_offset)?;
    let mut decoder = ArenaDecoder::new(ctx, bytes, strings);
    decoder.decode_value(document.root)
}

struct StringCollector<'a> {
    ctx: &'a Context,
    strings: Vec<Atom>,
    string_index: HashMap<Atom, usize>,
    visited: HashSet<usize>,
    stack: Vec<usize>,
    array_count: usize,
    object_count: usize,
    estimated_heap_bytes: usize,
    has_shared_references: bool,
}

impl<'a> StringCollector<'a> {
    #[inline(always)]
    fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
            strings: Vec::new(),
            string_index: HashMap::new(),
            visited: HashSet::new(),
            stack: Vec::with_capacity(32),
            array_count: 0,
            object_count: 0,
            estimated_heap_bytes: 0,
            has_shared_references: false,
        }
    }

    #[inline(always)]
    fn collect(&mut self, value: Value) -> Result<(), ValueError> {
        if let Some(atom) = value.as_atom() {
            self.intern(atom);
            return Ok(());
        }

        if value.is_null()
            || value.is_undefined()
            || value.as_bool().is_some()
            || value.as_i32().is_some()
        {
            return Ok(());
        }

        if let Some(float) = value.as_f64() {
            if !float.is_finite() {
                return Err(ValueError::NonFiniteFloat(float));
            }
            return Ok(());
        }

        match value.heap_kind() {
            Some(HeapKind::Array) => {
                let array = Gc::<QArray>::try_from(value)
                    .map_err(|_| ValueError::InvalidValueShape("invalid array value"))?;
                let ptr = heap_ptr(Value::from(&array), "array")?;
                if self.visited.contains(&ptr) {
                    self.has_shared_references = true;
                    return Ok(());
                }
                if self.stack.contains(&ptr) {
                    return Err(ValueError::InvalidValueShape(
                        "cyclic array cannot be converted to arena buffer",
                    ));
                }
                self.stack.push(ptr);
                let array_ref = array.borrow();
                self.array_count += 1;
                self.estimated_heap_bytes = self
                    .estimated_heap_bytes
                    .saturating_add(size_of::<ArrayData>())
                    .saturating_add(
                        array_ref
                            .elements
                            .len()
                            .saturating_mul(size_of::<SerializedValue>()),
                    );
                for &item in &array_ref.elements {
                    self.collect(item)?;
                }
                self.stack.pop();
                self.visited.insert(ptr);
                Ok(())
            }
            Some(HeapKind::Object) => {
                let object = Gc::<QObject>::try_from(value)
                    .map_err(|_| ValueError::InvalidValueShape("invalid object value"))?;
                let ptr = heap_ptr(Value::from(&object), "object")?;
                if self.visited.contains(&ptr) {
                    self.has_shared_references = true;
                    return Ok(());
                }
                if self.stack.contains(&ptr) {
                    return Err(ValueError::InvalidValueShape(
                        "cyclic object cannot be converted to arena buffer",
                    ));
                }
                self.stack.push(ptr);
                let object_ref = object.borrow();
                self.object_count += 1;
                let props = object_ref.shape.sorted_props();
                self.estimated_heap_bytes = self
                    .estimated_heap_bytes
                    .saturating_add(size_of::<ObjectData>())
                    .saturating_add(props.len().saturating_mul(size_of::<ObjectEntry>()));
                for &(atom, index) in props.iter() {
                    self.intern(atom);
                    let item = object_ref
                        .values
                        .get(index)
                        .copied()
                        .unwrap_or_else(Value::undefined);
                    self.collect(item)?;
                }
                self.stack.pop();
                self.visited.insert(ptr);
                Ok(())
            }
            Some(HeapKind::String) => {
                let string = Gc::<QString>::try_from(value)
                    .map_err(|_| ValueError::InvalidValueShape("invalid string object"))?;
                self.intern(string.borrow().atom);
                Ok(())
            }
            Some(kind) => Err(ValueError::InvalidValueShape(match kind {
                HeapKind::BoolArray => "bool arrays are not supported by arena buffer",
                HeapKind::Uint8Array => "uint8 arrays are not supported by arena buffer",
                HeapKind::Int32Array => "int32 arrays are not supported by arena buffer",
                HeapKind::Float64Array => "float64 arrays are not supported by arena buffer",
                HeapKind::StringArray => "string arrays are not supported by arena buffer",
                HeapKind::Symbol => "symbols are not supported by arena buffer",
                HeapKind::Function => "functions are not supported by arena buffer",
                HeapKind::Closure => "closures are not supported by arena buffer",
                HeapKind::NativeFunction => "native functions are not supported by arena buffer",
                HeapKind::NativeClosure => "native closures are not supported by arena buffer",
                HeapKind::Class => "classes are not supported by arena buffer",
                HeapKind::Module => "modules are not supported by arena buffer",
                HeapKind::Instance => "instances are not supported by arena buffer",
                _ => "unsupported heap kind",
            })),
            None => Err(ValueError::InvalidValueShape(
                "unknown value cannot be converted to arena buffer",
            )),
        }
    }

    #[inline(always)]
    fn intern(&mut self, atom: Atom) {
        if self.string_index.contains_key(&atom) {
            return;
        }
        let index = self.strings.len();
        self.strings.push(atom);
        self.string_index.insert(atom, index);
        let _ = self.ctx;
    }
}

struct ArenaEncoder {
    string_index: HashMap<Atom, usize>,
    heap_base: usize,
    heap: Vec<u8>,
    arrays: HashMap<usize, usize>,
    objects: HashMap<usize, usize>,
    track_shared_references: bool,
}

impl ArenaEncoder {
    #[inline(always)]
    fn new(
        string_index: HashMap<Atom, usize>,
        heap_base: usize,
        estimated_heap_bytes: usize,
        array_count: usize,
        object_count: usize,
        track_shared_references: bool,
    ) -> Self {
        Self {
            string_index,
            heap_base,
            heap: Vec::with_capacity(estimated_heap_bytes.max(256)),
            arrays: HashMap::with_capacity(if track_shared_references {
                array_count
            } else {
                0
            }),
            objects: HashMap::with_capacity(if track_shared_references {
                object_count
            } else {
                0
            }),
            track_shared_references,
        }
    }

    #[inline(always)]
    fn encode_value(&mut self, value: Value) -> Result<SerializedValue, ValueError> {
        if value.is_null() {
            return Ok(SerializedValue {
                tag: TAG_NULL,
                repr: ValueRepr { null: 0 },
            });
        }

        if let Some(value) = value.as_bool() {
            return Ok(SerializedValue {
                tag: TAG_BOOL,
                repr: ValueRepr { bool_: value },
            });
        }

        if let Some(value) = value.as_i32() {
            return Ok(SerializedValue {
                tag: TAG_I32,
                repr: ValueRepr { i32_: value },
            });
        }

        if let Some(value) = value.as_f64() {
            if !value.is_finite() {
                return Err(ValueError::NonFiniteFloat(value));
            }
            return Ok(SerializedValue {
                tag: TAG_F64,
                repr: ValueRepr { f64_: value },
            });
        }

        if let Some(atom) = value.as_atom() {
            return Ok(SerializedValue {
                tag: TAG_STRING,
                repr: ValueRepr {
                    string: StringIndex(self.lookup_string(atom)?),
                },
            });
        }

        match value.heap_kind() {
            Some(HeapKind::Array) => {
                let array = Gc::<QArray>::try_from(value)
                    .map_err(|_| ValueError::InvalidValueShape("invalid array value"))?;
                let offset = self.encode_array(array)?;
                Ok(SerializedValue {
                    tag: TAG_ARRAY,
                    repr: ValueRepr {
                        array: OffsetArray(offset),
                    },
                })
            }
            Some(HeapKind::Object) => {
                let object = Gc::<QObject>::try_from(value)
                    .map_err(|_| ValueError::InvalidValueShape("invalid object value"))?;
                let offset = self.encode_object(object)?;
                Ok(SerializedValue {
                    tag: TAG_OBJECT,
                    repr: ValueRepr {
                        object: OffsetObject(offset),
                    },
                })
            }
            Some(HeapKind::String) => {
                let string = Gc::<QString>::try_from(value)
                    .map_err(|_| ValueError::InvalidValueShape("invalid string object"))?;
                let atom = string.borrow().atom;
                Ok(SerializedValue {
                    tag: TAG_STRING,
                    repr: ValueRepr {
                        string: StringIndex(self.lookup_string(atom)?),
                    },
                })
            }
            Some(HeapKind::Function) => Err(ValueError::InvalidValueShape(
                "functions are not supported by arena buffer",
            )),
            Some(HeapKind::Closure) => Err(ValueError::InvalidValueShape(
                "closures are not supported by arena buffer",
            )),
            Some(HeapKind::Class) => Err(ValueError::InvalidValueShape(
                "classes are not supported by arena buffer",
            )),
            Some(HeapKind::Module) => Err(ValueError::InvalidValueShape(
                "modules are not supported by arena buffer",
            )),
            Some(_) => Err(ValueError::InvalidValueShape(
                "heap kind is not supported by arena buffer",
            )),
            None => Err(ValueError::InvalidValueShape(
                "unknown value cannot be converted to arena buffer",
            )),
        }
    }

    #[inline(always)]
    fn encode_array(&mut self, array: Gc<QArray>) -> Result<usize, ValueError> {
        let ptr = heap_ptr(Value::from(&array), "array")?;
        if self.track_shared_references
            && let Some(&offset) = self.arrays.get(&ptr)
        {
            return Ok(offset);
        }

        let elements = {
            let array_ref = array.borrow();
            array_ref.elements.clone()
        };

        let mut encoded = Vec::with_capacity(elements.len());
        for value in elements {
            encoded.push(self.encode_value(value)?);
        }

        let offset = checked_add(
            self.heap_base,
            align_up(self.heap.len(), align_of::<ArrayData>()),
        )?;
        append_aligned_struct(&mut self.heap, &ArrayData { len: encoded.len() })?;
        for value in encoded {
            append_aligned_struct(&mut self.heap, &value)?;
        }

        if self.track_shared_references {
            self.arrays.insert(ptr, offset);
        }
        Ok(offset)
    }

    #[inline(always)]
    fn encode_object(&mut self, object: Gc<QObject>) -> Result<usize, ValueError> {
        let ptr = heap_ptr(Value::from(&object), "object")?;
        if self.track_shared_references
            && let Some(&offset) = self.objects.get(&ptr)
        {
            return Ok(offset);
        }

        let entries = {
            let object_ref = object.borrow();
            let props = object_ref.shape.sorted_props();
            let mut entries = Vec::with_capacity(props.len());
            for &(atom, index) in props.iter() {
                entries.push((
                    atom,
                    object_ref
                        .values
                        .get(index)
                        .copied()
                        .unwrap_or_else(Value::undefined),
                ));
            }
            entries
        };

        let mut encoded = Vec::with_capacity(entries.len());
        for (atom, value) in entries {
            encoded.push(ObjectEntry {
                key: StringIndex(self.lookup_string(atom)?),
                value: self.encode_value(value)?,
            });
        }

        let offset = checked_add(
            self.heap_base,
            align_up(self.heap.len(), align_of::<ObjectData>()),
        )?;
        append_aligned_struct(&mut self.heap, &ObjectData { len: encoded.len() })?;
        for entry in encoded {
            append_aligned_struct(&mut self.heap, &entry)?;
        }

        if self.track_shared_references {
            self.objects.insert(ptr, offset);
        }
        Ok(offset)
    }

    #[inline(always)]
    fn lookup_string(&self, atom: Atom) -> Result<usize, ValueError> {
        self.string_index
            .get(&atom)
            .copied()
            .ok_or(ValueError::InvalidValueShape(
                "string table index missing during arena encoding",
            ))
    }
}

struct ArenaDecoder<'a> {
    ctx: &'a Context,
    bytes: &'a [u8],
    strings: Vec<Atom>,
    arrays: HashMap<usize, Value>,
    objects: HashMap<usize, Value>,
    stack: Vec<usize>,
}

impl<'a> ArenaDecoder<'a> {
    #[inline(always)]
    fn new(ctx: &'a Context, bytes: &'a [u8], strings: Vec<Atom>) -> Self {
        Self {
            ctx,
            bytes,
            strings,
            arrays: HashMap::new(),
            objects: HashMap::new(),
            stack: Vec::with_capacity(32),
        }
    }

    #[inline(always)]
    fn decode_value(&mut self, value: SerializedValue) -> Result<Value, ValueError> {
        match value.tag.0 {
            0 => Ok(Value::null()),
            1 => Ok(Value::from(unsafe { value.repr.bool_ })),
            2 => Ok(Value::from(unsafe { value.repr.i32_ })),
            3 => {
                let value = unsafe { value.repr.f64_ };
                if !value.is_finite() {
                    return Err(ValueError::NonFiniteFloat(value));
                }
                Ok(Value::from(value))
            }
            4 => {
                let index = unsafe { value.repr.string }.0;
                let atom = *self
                    .strings
                    .get(index)
                    .ok_or(ValueError::InvalidValueShape(
                        "string index out of bounds in arena buffer",
                    ))?;
                Ok(Value::from(atom))
            }
            5 => self.decode_array(unsafe { value.repr.array }.0),
            6 => self.decode_object(unsafe { value.repr.object }.0),
            tag => Err(ValueError::InvalidTag(tag)),
        }
    }

    #[inline(always)]
    fn decode_array(&mut self, offset: usize) -> Result<Value, ValueError> {
        if let Some(&value) = self.arrays.get(&offset) {
            return Ok(value);
        }
        if self.stack.contains(&offset) {
            return Err(ValueError::InvalidValueShape(
                "cyclic arena array reference",
            ));
        }

        let header = read_struct::<ArrayData>(self.bytes, offset)?;
        let array = self.ctx.new_array();
        let value = Value::from(array.clone());
        self.arrays.insert(offset, value);
        self.stack.push(offset);

        let mut cursor = checked_add(offset, size_of::<ArrayData>())?;
        let mut array_ref = array.borrow_mut();
        for _ in 0..header.len {
            cursor = align_up(cursor, align_of::<SerializedValue>());
            let item = read_struct::<SerializedValue>(self.bytes, cursor)?;
            cursor = checked_add(cursor, size_of::<SerializedValue>())?;
            array_ref.push(self.decode_value(item)?);
        }
        drop(array_ref);

        self.stack.pop();
        Ok(value)
    }

    #[inline(always)]
    fn decode_object(&mut self, offset: usize) -> Result<Value, ValueError> {
        if let Some(&value) = self.objects.get(&offset) {
            return Ok(value);
        }
        if self.stack.contains(&offset) {
            return Err(ValueError::InvalidValueShape(
                "cyclic arena object reference",
            ));
        }

        let header = read_struct::<ObjectData>(self.bytes, offset)?;
        let object = self.ctx.new_object();
        let value = Value::from(object.clone());
        self.objects.insert(offset, value);
        self.stack.push(offset);

        let mut cursor = checked_add(offset, size_of::<ObjectData>())?;
        let mut object_ref = object.borrow_mut();
        for _ in 0..header.len {
            cursor = align_up(cursor, align_of::<ObjectEntry>());
            let entry = read_struct::<ObjectEntry>(self.bytes, cursor)?;
            cursor = checked_add(cursor, size_of::<ObjectEntry>())?;

            let atom = *self
                .strings
                .get(entry.key.0)
                .ok_or(ValueError::InvalidValueShape(
                    "object key string index out of bounds",
                ))?;
            let decoded = self.decode_value(entry.value)?;
            object_ref.set(atom, decoded);
        }
        drop(object_ref);

        self.stack.pop();
        Ok(value)
    }
}

#[inline(always)]
fn encode_string_table(ctx: &Context, strings: &[Atom]) -> Result<Vec<u8>, ValueError> {
    let mut out = Vec::with_capacity(128);
    append_aligned_struct(&mut out, &StringTable { len: strings.len() })?;

    for &atom in strings {
        ctx.with_resolved(atom, |text| -> Result<(), ValueError> {
            let bytes = text.as_bytes();
            let len =
                u16::try_from(bytes.len()).map_err(|_| ValueError::StringTooLong(bytes.len()))?;
            append_aligned_struct(&mut out, &StringData { len })?;
            out.extend_from_slice(bytes);
            Ok(())
        })?;
    }

    Ok(out)
}

#[inline(always)]
fn parse_string_table(ctx: &Context, bytes: &[u8], offset: usize) -> Result<Vec<Atom>, ValueError> {
    let table = read_struct::<StringTable>(bytes, offset)?;
    let mut strings = Vec::with_capacity(table.len);
    let mut cursor = checked_add(offset, size_of::<StringTable>())?;

    for _ in 0..table.len {
        cursor = align_up(cursor, align_of::<StringData>());
        let data = read_struct::<StringData>(bytes, cursor)?;
        cursor = checked_add(cursor, size_of::<StringData>())?;
        let len = usize::from(data.len);
        let end = checked_add(cursor, len)?;
        let slice = bytes.get(cursor..end).ok_or(ValueError::OutOfBounds {
            offset: cursor,
            len,
            arena_len: bytes.len(),
        })?;
        let text = std::str::from_utf8(slice)?;
        strings.push(ctx.intern(text));
        cursor = end;
    }

    Ok(strings)
}

#[inline(always)]
fn append_padding(buf: &mut Vec<u8>, target_len: usize) {
    if buf.len() < target_len {
        buf.resize(target_len, 0);
    }
}

#[inline(always)]
fn append_aligned_struct<T: Copy>(buf: &mut Vec<u8>, value: &T) -> Result<(), ValueError> {
    let aligned = align_up(buf.len(), align_of::<T>());
    append_padding(buf, aligned);
    let bytes =
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    let new_len = checked_add(buf.len(), bytes.len())?;
    buf.reserve(new_len.saturating_sub(buf.len()));
    buf.extend_from_slice(bytes);
    Ok(())
}

#[inline(always)]
fn write_struct_at<T: Copy>(buf: &mut Vec<u8>, offset: usize, value: &T) -> Result<(), ValueError> {
    let end = checked_add(offset, size_of::<T>())?;
    append_padding(buf, end);
    let bytes =
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    buf[offset..end].copy_from_slice(bytes);
    Ok(())
}

#[inline(always)]
fn read_struct<T: Copy>(bytes: &[u8], offset: usize) -> Result<T, ValueError> {
    if !offset.is_multiple_of(align_of::<T>()) {
        return Err(ValueError::MisalignedOffset {
            offset,
            align: align_of::<T>(),
        });
    }

    let len = size_of::<T>();
    let end = checked_add(offset, len)?;
    let slice = bytes.get(offset..end).ok_or(ValueError::OutOfBounds {
        offset,
        len,
        arena_len: bytes.len(),
    })?;

    let mut out = MaybeUninit::<T>::uninit();
    unsafe {
        std::ptr::copy_nonoverlapping(slice.as_ptr(), out.as_mut_ptr().cast::<u8>(), len);
        Ok(out.assume_init())
    }
}

#[inline(always)]
fn heap_ptr(value: Value, type_name: &'static str) -> Result<usize, ValueError> {
    value
        .as_heap_ptr()
        .map(|ptr| ptr as usize)
        .ok_or(ValueError::InvalidValueShape(match type_name {
            "array" => "invalid array heap pointer",
            "object" => "invalid object heap pointer",
            _ => "invalid heap pointer",
        }))
}

#[inline(always)]
fn heap_align() -> usize {
    align_of::<ObjectEntry>()
        .max(align_of::<ObjectData>())
        .max(align_of::<ArrayData>())
        .max(align_of::<SerializedValue>())
}

#[inline(always)]
fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + (align - 1)) & !(align - 1)
}

#[inline(always)]
fn checked_add(left: usize, right: usize) -> Result<usize, ValueError> {
    left.checked_add(right).ok_or(ValueError::SizeOverflow)
}
