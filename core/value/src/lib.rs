pub use atoms::Atom;
pub use gc::{AtomTrace, GCHeader, Gc, GcBox, HeapKind, HeapTyped, ObjType, Trace};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Null;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Undefined;

pub const NULL: Null = Null;
pub const UNDEFINED: Undefined = Undefined;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueTypeError {
    expected: &'static str,
    actual: &'static str,
}

impl ValueTypeError {
    pub(crate) fn new(expected: &'static str, actual: &'static str) -> Self {
        Self { expected, actual }
    }

    pub fn expected(&self) -> &'static str {
        self.expected
    }

    pub fn actual(&self) -> &'static str {
        self.actual
    }
}

impl std::fmt::Display for ValueTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expected {}, got {}", self.expected, self.actual)
    }
}

impl std::error::Error for ValueTypeError {}

const QNAN: u64 = 0x7ffc_0000_0000_0000;
const TAG_MASK: u64 = 0xf;
const PAYLOAD_MASK: u64 = 0x0000_ffff_ffff_fff0;

const TAG_INT: u64 = 1;
const TAG_BOOL: u64 = 2;
const TAG_NULL: u64 = 3;
const TAG_UNDEF: u64 = 4;
const TAG_HEAP: u64 = 5;
const TAG_ATOM: u64 = 6;
const TAG_EMPTY: u64 = 7;

impl Value {
    #[inline(always)]
    pub fn bits(self) -> u64 {
        self.0
    }

    #[inline(always)]
    fn tagged(tag: u64, payload: u64) -> Self {
        Self(QNAN | (payload & PAYLOAD_MASK) | tag)
    }

    #[inline(always)]
    fn is_tagged(self) -> bool {
        (self.0 & QNAN) == QNAN
    }

    #[inline(always)]
    fn tag(self) -> u64 {
        self.0 & TAG_MASK
    }

    #[inline(always)]
    fn int_payload(bits: u64) -> i32 {
        (bits >> 4) as i32
    }

    #[inline(always)]
    pub fn heap(ptr: *const GCHeader) -> Self {
        debug_assert_eq!(
            (ptr as usize) & 0xf,
            0,
            "Heap pointer must be 16-byte aligned for safe tagging"
        );
        debug_assert_eq!((ptr as usize) & TAG_MASK as usize, 0);
        Self::tagged(TAG_HEAP, (ptr as usize as u64) & PAYLOAD_MASK)
    }

    #[inline(always)]
    pub fn as_heap_ptr(self) -> Option<*const GCHeader> {
        if !(self.is_tagged() && self.tag() == TAG_HEAP) {
            return None;
        }

        Some((self.0 & PAYLOAD_MASK) as *const GCHeader)
    }

    #[inline(always)]
    pub fn i32(v: i32) -> Self {
        Self::tagged(TAG_INT, ((v as u32) as u64) << 4)
    }

    #[inline(always)]
    pub fn bool(v: bool) -> Self {
        Self::tagged(TAG_BOOL, (u64::from(v)) << 4)
    }

    #[inline(always)]
    pub fn f64(v: f64) -> Self {
        let bits = v.to_bits();
        if bits & QNAN == QNAN {
            Self(f64::NAN.to_bits())
        } else {
            Self(bits)
        }
    }

    #[inline(always)]
    pub fn null() -> Self {
        Self::tagged(TAG_NULL, 0)
    }

    #[inline(always)]
    pub fn undefined() -> Self {
        Self::tagged(TAG_UNDEF, 0)
    }

    #[inline(always)]
    pub fn empty() -> Self {
        Self::tagged(TAG_EMPTY, 0)
    }

    #[inline(always)]
    pub fn atom(atom: Atom) -> Self {
        Self::tagged(TAG_ATOM, (atom.0 as u64) << 4)
    }

    #[inline(always)]
    pub fn as_i32(self) -> Option<i32> {
        if self.is_tagged() && self.tag() == TAG_INT {
            Some(((self.0 >> 4) as u32) as i32)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_bool(self) -> Option<bool> {
        if self.is_tagged() && self.tag() == TAG_BOOL {
            Some(((self.0 >> 4) & 1) != 0)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_f64(self) -> Option<f64> {
        if self.is_tagged() {
            None
        } else {
            Some(f64::from_bits(self.0))
        }
    }

    #[inline(always)]
    pub fn is_int(self) -> bool {
        self.is_tagged() && self.tag() == TAG_INT
    }

    #[inline(always)]
    pub fn int_payload_unchecked(self) -> i32 {
        Self::int_payload(self.0)
    }

    #[inline(always)]
    pub fn is_f64(self) -> bool {
        !self.is_tagged()
    }

    #[inline(always)]
    pub fn f64_payload_unchecked(self) -> f64 {
        f64::from_bits(self.0)
    }

    #[inline(always)]
    pub fn as_atom(self) -> Option<Atom> {
        if self.is_tagged() && self.tag() == TAG_ATOM {
            Some(Atom((self.0 >> 4) as u32))
        } else {
            None
        }
    }

    #[inline]
    pub fn heap_kind(self) -> Option<HeapKind> {
        self.as_heap_ptr().map(|ptr| unsafe { (*ptr).kind })
    }

    #[inline]
    pub fn obj_type(self) -> Option<ObjType> {
        self.as_heap_ptr().map(|ptr| unsafe { (*ptr).obj_type })
    }

    #[inline]
    pub fn type_name(self) -> &'static str {
        let bits = self.0;

        if (bits & QNAN) != QNAN {
            return "number";
        }

        match bits & TAG_MASK {
            TAG_INT => "number",
            TAG_BOOL => "bool",
            TAG_NULL => "null",
            TAG_UNDEF | TAG_EMPTY => "undefined",
            TAG_ATOM => "string",
            TAG_HEAP => self.heap_kind().map_or("object", HeapKind::type_name),
            _ => "unknown",
        }
    }

    #[inline]
    fn integer_value(self) -> Option<i128> {
        if let Some(value) = self.as_i32() {
            return Some(value as i128);
        }

        let value = self.as_f64()?;
        if !value.is_finite() || value.fract() != 0.0 {
            return None;
        }

        Some(value as i128)
    }

    #[inline]
    fn unsigned_integer_value(self) -> Option<u128> {
        self.integer_value()
            .and_then(|value| u128::try_from(value).ok())
    }

    #[inline(always)]
    pub fn is_null(self) -> bool {
        self.is_tagged() && self.tag() == TAG_NULL
    }

    #[inline(always)]
    pub fn is_undefined(self) -> bool {
        self.is_tagged() && matches!(self.tag(), TAG_UNDEF | TAG_EMPTY)
    }

    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.is_tagged() && self.tag() == TAG_EMPTY
    }

    #[inline(always)]
    pub fn is_heap(self) -> bool {
        self.is_tagged() && self.tag() == TAG_HEAP
    }

    #[inline]
    pub fn is_truthy(self) -> bool {
        let bits = self.0;

        if (bits & QNAN) != QNAN {
            let f = f64::from_bits(bits);
            return f != 0.0 && !f.is_nan();
        }

        match bits & TAG_MASK {
            TAG_BOOL => ((bits >> 4) & 1) != 0,
            TAG_INT => Self::int_payload(bits) != 0,
            TAG_NULL | TAG_UNDEF | TAG_EMPTY => false,
            _ => true,
        }
    }

    #[inline(always)]
    pub fn to_number_ecma(self) -> f64 {
        let bits = self.0;

        if (bits & QNAN) != QNAN {
            return f64::from_bits(bits);
        }

        match bits & TAG_MASK {
            TAG_INT => Self::int_payload(bits) as f64,
            TAG_BOOL => {
                if ((bits >> 4) & 1) != 0 {
                    1.0
                } else {
                    0.0
                }
            }
            TAG_NULL => 0.0,
            TAG_UNDEF | TAG_EMPTY => f64::NAN,
            _ => f64::NAN,
        }
    }

    #[inline(always)]
    pub fn to_i32_ecma(self) -> i32 {
        let bits = self.0;

        if (bits & QNAN) != QNAN {
            let f = f64::from_bits(bits);
            if f.is_nan() || f.is_infinite() {
                return 0;
            }
            return f as i32;
        }

        match bits & TAG_MASK {
            TAG_INT => Self::int_payload(bits),
            TAG_BOOL => {
                if ((bits >> 4) & 1) != 0 {
                    1
                } else {
                    0
                }
            }
            TAG_NULL => 0,
            TAG_UNDEF | TAG_EMPTY => 0,
            _ => 0,
        }
    }

    #[inline(always)]
    pub fn is_object(self) -> bool {
        self.obj_type() == Some(ObjType::Object)
    }

    #[inline(always)]
    pub fn is_string(self) -> bool {
        self.as_atom().is_some() || self.obj_type() == Some(ObjType::String)
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(v) = self.as_i32() {
            return f.debug_tuple("Int").field(&v).finish();
        }
        if let Some(v) = self.as_bool() {
            return f.debug_tuple("Bool").field(&v).finish();
        }
        if let Some(v) = self.as_f64() {
            return f.debug_tuple("Float").field(&v).finish();
        }
        if let Some(v) = self.as_atom() {
            return f.debug_tuple("Atom").field(&v.0).finish();
        }
        if let Some(kind) = self.heap_kind() {
            return f.debug_tuple("Heap").field(&kind).finish();
        }
        if self.is_null() {
            return f.write_str("Null");
        }
        if self.is_undefined() {
            return f.write_str("Undefined");
        }
        f.write_str("Value(?)")
    }
}

impl From<Null> for Value {
    #[inline(always)]
    fn from(_: Null) -> Self {
        Self::null()
    }
}

impl From<Undefined> for Value {
    #[inline(always)]
    fn from(_: Undefined) -> Self {
        Self::undefined()
    }
}

impl From<()> for Value {
    #[inline(always)]
    fn from(_: ()) -> Self {
        Self::null()
    }
}

impl From<bool> for Value {
    #[inline(always)]
    fn from(value: bool) -> Self {
        Self::bool(value)
    }
}

macro_rules! impl_exact_int_value_from {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl From<$ty> for Value {
                #[inline(always)]
                fn from(value: $ty) -> Self {
                    Self::i32(i32::from(value))
                }
            }
        )+
    };
}

macro_rules! impl_fallible_int_value_from {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl From<$ty> for Value {
                #[inline]
                fn from(value: $ty) -> Self {
                    match i32::try_from(value) {
                        Ok(value) => Self::i32(value),
                        Err(_) => Self::f64(value as f64),
                    }
                }
            }
        )+
    };
}

macro_rules! impl_float_value_from {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl From<$ty> for Value {
                #[inline(always)]
                fn from(value: $ty) -> Self {
                    Self::f64(value as f64)
                }
            }
        )+
    };
}

impl_exact_int_value_from!(i8, i16, i32, u8, u16);
impl_fallible_int_value_from!(isize, i64, i128, u32, usize, u64, u128);
impl_float_value_from!(f32, f64);

impl From<Atom> for Value {
    #[inline(always)]
    fn from(value: Atom) -> Self {
        Self::atom(value)
    }
}

impl From<&Atom> for Value {
    #[inline(always)]
    fn from(value: &Atom) -> Self {
        Self::atom(*value)
    }
}

impl<T> From<Gc<T>> for Value {
    #[inline(always)]
    fn from(value: Gc<T>) -> Self {
        Self::heap(value.header_ptr())
    }
}

impl<T> From<&Gc<T>> for Value {
    #[inline(always)]
    fn from(value: &Gc<T>) -> Self {
        Self::heap(value.header_ptr())
    }
}

impl TryFrom<Value> for Null {
    type Error = ValueTypeError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        value
            .is_null()
            .then_some(Null)
            .ok_or_else(|| ValueTypeError::new("null", value.type_name()))
    }
}

impl TryFrom<Value> for Undefined {
    type Error = ValueTypeError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        value
            .is_undefined()
            .then_some(Undefined)
            .ok_or_else(|| ValueTypeError::new("undefined", value.type_name()))
    }
}

impl TryFrom<Value> for bool {
    type Error = ValueTypeError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        value
            .as_bool()
            .ok_or_else(|| ValueTypeError::new("bool", value.type_name()))
    }
}

macro_rules! impl_try_from_signed_int {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl TryFrom<Value> for $ty {
                type Error = ValueTypeError;

                fn try_from(value: Value) -> Result<Self, Self::Error> {
                    let integer = value
                        .integer_value()
                        .ok_or_else(|| ValueTypeError::new("number", value.type_name()))?;

                    <$ty>::try_from(integer)
                        .map_err(|_| ValueTypeError::new(stringify!($ty), value.type_name()))
                }
            }
        )+
    };
}

macro_rules! impl_try_from_unsigned_int {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl TryFrom<Value> for $ty {
                type Error = ValueTypeError;

                fn try_from(value: Value) -> Result<Self, Self::Error> {
                    let integer = value
                        .unsigned_integer_value()
                        .ok_or_else(|| ValueTypeError::new("number", value.type_name()))?;

                    <$ty>::try_from(integer)
                        .map_err(|_| ValueTypeError::new(stringify!($ty), value.type_name()))
                }
            }
        )+
    };
}

macro_rules! impl_try_from_float {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl TryFrom<Value> for $ty {
                type Error = ValueTypeError;

                fn try_from(value: Value) -> Result<Self, Self::Error> {
                    if let Some(integer) = value.as_i32() {
                        return Ok(integer as $ty);
                    }

                    value
                        .as_f64()
                        .map(|v| v as $ty)
                        .ok_or_else(|| ValueTypeError::new("number", value.type_name()))
                }
            }
        )+
    };
}

impl_try_from_signed_int!(i8, i16, i32, i64, i128, isize);
impl_try_from_unsigned_int!(u8, u16, u32, u64, u128, usize);
impl_try_from_float!(f32, f64);

impl TryFrom<Value> for Atom {
    type Error = ValueTypeError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        value
            .as_atom()
            .ok_or_else(|| ValueTypeError::new("string", value.type_name()))
    }
}

impl<T: Trace + AtomTrace + HeapTyped + 'static> TryFrom<Value> for Gc<T> {
    type Error = ValueTypeError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let ptr = value
            .as_heap_ptr()
            .ok_or_else(|| ValueTypeError::new(T::KIND.type_name(), value.type_name()))?;

        let kind = unsafe { (*ptr).kind };
        if kind != T::KIND {
            return Err(ValueTypeError::new(T::KIND.type_name(), kind.type_name()));
        }

        let ptr = ptr as *const GcBox<T>;
        unsafe { Ok(Gc::clone_from_raw(ptr)) }
    }
}

pub type JSValue = Value;

#[inline(always)]
pub fn make_int32(value: i32) -> JSValue {
    Value::i32(value)
}

#[inline(always)]
pub fn make_number(value: f64) -> JSValue {
    Value::f64(value)
}

#[inline(always)]
pub fn make_bool(value: bool) -> JSValue {
    Value::bool(value)
}

#[inline(always)]
pub fn make_true() -> JSValue {
    Value::bool(true)
}

#[inline(always)]
pub fn make_false() -> JSValue {
    Value::bool(false)
}

#[inline(always)]
pub fn make_null() -> JSValue {
    Value::null()
}

#[inline(always)]
pub fn make_undefined() -> JSValue {
    Value::undefined()
}

#[inline(always)]
pub fn make_heap(ptr: *const GCHeader) -> JSValue {
    Value::heap(ptr)
}

#[inline(always)]
pub fn to_f64(value: JSValue) -> Option<f64> {
    value.as_i32().map(f64::from).or_else(|| value.as_f64())
}

#[inline(always)]
pub fn to_i32(value: JSValue) -> Option<i32> {
    value
        .as_i32()
        .or_else(|| value.as_f64().map(|number| number as i32))
}

#[inline(always)]
pub fn bool_from_value(value: JSValue) -> Option<bool> {
    value.as_bool()
}

#[inline(always)]
pub fn is_number(value: JSValue) -> bool {
    value.as_i32().is_some() || value.as_f64().is_some()
}

#[inline(always)]
pub fn is_null(value: JSValue) -> bool {
    value.is_null()
}

#[inline(always)]
pub fn is_undefined(value: JSValue) -> bool {
    value.is_undefined()
}

#[inline(always)]
pub fn is_truthy(value: JSValue) -> bool {
    value.is_truthy()
}

#[inline(always)]
pub fn object_from_value(value: JSValue) -> Option<*const GCHeader> {
    let ptr = value.as_heap_ptr()?;
    (unsafe { (*ptr).obj_type } == ObjType::Object).then_some(ptr)
}

#[inline(always)]
pub fn string_from_value(value: JSValue) -> Option<*const GCHeader> {
    let ptr = value.as_heap_ptr()?;
    (unsafe { (*ptr).obj_type } == ObjType::String).then_some(ptr)
}

#[inline(always)]
pub fn is_object(value: JSValue) -> bool {
    object_from_value(value).is_some()
}

#[inline(always)]
pub fn is_string(value: JSValue) -> bool {
    value.as_atom().is_some() || string_from_value(value).is_some()
}
