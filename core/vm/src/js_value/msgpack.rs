use crate::atoms::Atom;
use crate::gc::Gc;
use crate::heap::{
    QArray, QBoolArray, QFloat64Array, QInt32Array, QObject, QString, QStringArray, QUint8Array,
};
use crate::runtime::Context;

use super::{HeapKind, MsgpackValueError, Value};

pub(super) fn from_msgpack(ctx: &Context, bytes: &[u8]) -> Result<Value, MsgpackValueError> {
    MsgpackParser::new(ctx, bytes).parse()
}

pub(super) fn to_msgpack(ctx: &Context, value: Value) -> Result<Vec<u8>, MsgpackValueError> {
    let mut out = Vec::with_capacity(256);
    let mut seen = Vec::with_capacity(32);
    write_msgpack_value(&mut out, ctx, value, &mut seen)?;
    Ok(out)
}

fn write_msgpack_value(
    out: &mut Vec<u8>,
    ctx: &Context,
    value: Value,
    seen: &mut Vec<usize>,
) -> Result<(), MsgpackValueError> {
    if value.is_null() {
        out.push(0xc0);
        return Ok(());
    }

    if value.is_undefined() {
        return Err(MsgpackValueError::unsupported("undefined"));
    }

    if let Some(value) = value.as_bool() {
        out.push(if value { 0xc3 } else { 0xc2 });
        return Ok(());
    }

    if let Some(value) = value.as_i32() {
        write_msgpack_i64(out, value as i64);
        return Ok(());
    }

    if let Some(value) = value.as_f64() {
        if !value.is_finite() {
            return Err(MsgpackValueError::invalid_number(value));
        }
        out.push(0xcb);
        out.extend_from_slice(&value.to_bits().to_be_bytes());
        return Ok(());
    }

    if let Some(atom) = value.as_atom() {
        return ctx.with_resolved(atom, |text| write_msgpack_str(out, text));
    }

    match value.heap_kind() {
        Some(HeapKind::Object) => {
            write_msgpack_object(out, ctx, Gc::<QObject>::try_from(value).unwrap(), seen)
        }
        Some(HeapKind::Array) => {
            write_msgpack_array(out, ctx, Gc::<QArray>::try_from(value).unwrap(), seen)
        }
        Some(HeapKind::BoolArray) => {
            let array = Gc::<QBoolArray>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_msgpack_array_len(out, array_ref.elements.len());
            for &value in &array_ref.elements {
                out.push(if value { 0xc3 } else { 0xc2 });
            }
            Ok(())
        }
        Some(HeapKind::Uint8Array) => {
            let array = Gc::<QUint8Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_msgpack_array_len(out, array_ref.elements.len());
            for &value in &array_ref.elements {
                write_msgpack_u64(out, value as u64);
            }
            Ok(())
        }
        Some(HeapKind::Int32Array) => {
            let array = Gc::<QInt32Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_msgpack_array_len(out, array_ref.elements.len());
            for &value in &array_ref.elements {
                write_msgpack_i64(out, value as i64);
            }
            Ok(())
        }
        Some(HeapKind::Float64Array) => {
            let array = Gc::<QFloat64Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_msgpack_array_len(out, array_ref.elements.len());
            for &value in &array_ref.elements {
                if !value.is_finite() {
                    return Err(MsgpackValueError::invalid_number(value));
                }
                out.push(0xcb);
                out.extend_from_slice(&value.to_bits().to_be_bytes());
            }
            Ok(())
        }
        Some(HeapKind::StringArray) => {
            let array = Gc::<QStringArray>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_msgpack_array_len(out, array_ref.elements.len());
            for &value in &array_ref.elements {
                write_msgpack_value(out, ctx, value, seen)?;
            }
            Ok(())
        }
        Some(HeapKind::String) => {
            let string = Gc::<QString>::try_from(value).unwrap();
            let atom = string.borrow().atom;
            ctx.with_resolved(atom, |text| write_msgpack_str(out, text))
        }
        Some(kind) => Err(MsgpackValueError::unsupported(kind.type_name())),
        None => Err(MsgpackValueError::unsupported("unknown value")),
    }
}

fn write_msgpack_object(
    out: &mut Vec<u8>,
    ctx: &Context,
    object: Gc<QObject>,
    seen: &mut Vec<usize>,
) -> Result<(), MsgpackValueError> {
    let ptr = enter_heap(Value::from(&object), "object", seen)?;
    let result = (|| {
        let object_ref = object.borrow();
        write_msgpack_map_len(out, object_ref.shape.props.len());
        for (&atom, &index) in &object_ref.shape.props {
            ctx.with_resolved(atom, |text| write_msgpack_str(out, text))?;
            let value = object_ref
                .values
                .get(index)
                .copied()
                .unwrap_or_else(Value::undefined);
            write_msgpack_value(out, ctx, value, seen)?;
        }
        Ok(())
    })();
    leave_heap(ptr, seen);
    result
}

fn write_msgpack_array(
    out: &mut Vec<u8>,
    ctx: &Context,
    array: Gc<QArray>,
    seen: &mut Vec<usize>,
) -> Result<(), MsgpackValueError> {
    let ptr = enter_heap(Value::from(&array), "array", seen)?;
    let result = (|| {
        let array_ref = array.borrow();
        write_msgpack_array_len(out, array_ref.elements.len());
        for &value in &array_ref.elements {
            write_msgpack_value(out, ctx, value, seen)?;
        }
        Ok(())
    })();
    leave_heap(ptr, seen);
    result
}

fn write_msgpack_str(out: &mut Vec<u8>, text: &str) -> Result<(), MsgpackValueError> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    if len <= 31 {
        out.push(0xa0 | (len as u8));
    } else if u8::try_from(len).is_ok() {
        out.push(0xd9);
        out.push(len as u8);
    } else if u16::try_from(len).is_ok() {
        out.push(0xda);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else if u32::try_from(len).is_ok() {
        out.push(0xdb);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    } else {
        return Err(MsgpackValueError::parse("string too large for MsgPack"));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_msgpack_array_len(out: &mut Vec<u8>, len: usize) {
    if len <= 15 {
        out.push(0x90 | (len as u8));
    } else if u16::try_from(len).is_ok() {
        out.push(0xdc);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0xdd);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

fn write_msgpack_map_len(out: &mut Vec<u8>, len: usize) {
    if len <= 15 {
        out.push(0x80 | (len as u8));
    } else if u16::try_from(len).is_ok() {
        out.push(0xde);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0xdf);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

fn write_msgpack_u64(out: &mut Vec<u8>, value: u64) {
    if value <= 0x7f {
        out.push(value as u8);
    } else if u8::try_from(value).is_ok() {
        out.push(0xcc);
        out.push(value as u8);
    } else if u16::try_from(value).is_ok() {
        out.push(0xcd);
        out.extend_from_slice(&(value as u16).to_be_bytes());
    } else if u32::try_from(value).is_ok() {
        out.push(0xce);
        out.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        out.push(0xcf);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn write_msgpack_i64(out: &mut Vec<u8>, value: i64) {
    if (0..=0x7f).contains(&value) {
        out.push(value as u8);
    } else if (-32..=-1).contains(&value) {
        out.push(value as i8 as u8);
    } else if i8::try_from(value).is_ok() {
        out.push(0xd0);
        out.push(value as i8 as u8);
    } else if i16::try_from(value).is_ok() {
        out.push(0xd1);
        out.extend_from_slice(&(value as i16).to_be_bytes());
    } else if i32::try_from(value).is_ok() {
        out.push(0xd2);
        out.extend_from_slice(&(value as i32).to_be_bytes());
    } else {
        out.push(0xd3);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

fn enter_heap(
    value: Value,
    type_name: &'static str,
    seen: &mut Vec<usize>,
) -> Result<usize, MsgpackValueError> {
    let ptr = value
        .as_heap_ptr()
        .map(|ptr| ptr as usize)
        .ok_or_else(|| MsgpackValueError::unsupported(type_name))?;

    if seen.contains(&ptr) {
        return Err(MsgpackValueError::cyclic(type_name));
    }

    seen.push(ptr);

    Ok(ptr)
}

fn leave_heap(ptr: usize, seen: &mut Vec<usize>) {
    debug_assert_eq!(seen.last().copied(), Some(ptr));
    seen.pop();
}

struct MsgpackParser<'a> {
    ctx: &'a Context,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> MsgpackParser<'a> {
    fn new(ctx: &'a Context, bytes: &'a [u8]) -> Self {
        Self { ctx, bytes, pos: 0 }
    }

    fn parse(mut self) -> Result<Value, MsgpackValueError> {
        let value = self.parse_value()?;
        if self.pos != self.bytes.len() {
            return Err(MsgpackValueError::parse("trailing MsgPack data"));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<Value, MsgpackValueError> {
        let marker = self.read_u8()?;
        match marker {
            0x00..=0x7f => Ok(Value::from(marker as u64)),
            0x80..=0x8f => self.parse_map((marker & 0x0f) as usize),
            0x90..=0x9f => self.parse_array((marker & 0x0f) as usize),
            0xa0..=0xbf => self.parse_string((marker & 0x1f) as usize),
            0xc0 => Ok(Value::null()),
            0xc2 => Ok(Value::bool(false)),
            0xc3 => Ok(Value::bool(true)),
            0xcc => Ok(Value::from(self.read_u8()? as u64)),
            0xcd => Ok(Value::from(self.read_u16()? as u64)),
            0xce => Ok(Value::from(self.read_u32()? as u64)),
            0xcf => Ok(Value::from(self.read_u64()?)),
            0xd0 => Ok(Value::from(self.read_i8()? as i64)),
            0xd1 => Ok(Value::from(self.read_i16()? as i64)),
            0xd2 => Ok(Value::from(self.read_i32()? as i64)),
            0xd3 => Ok(Value::from(self.read_i64()?)),
            0xd9 => {
                let len = self.read_u8()? as usize;
                self.parse_string(len)
            }
            0xda => {
                let len = self.read_u16()? as usize;
                self.parse_string(len)
            }
            0xdb => {
                let len = self.read_u32()? as usize;
                self.parse_string(len)
            }
            0xdc => {
                let len = self.read_u16()? as usize;
                self.parse_array(len)
            }
            0xdd => {
                let len = self.read_u32()? as usize;
                self.parse_array(len)
            }
            0xde => {
                let len = self.read_u16()? as usize;
                self.parse_map(len)
            }
            0xdf => {
                let len = self.read_u32()? as usize;
                self.parse_map(len)
            }
            0xca => Ok(Value::from(f32::from_bits(self.read_u32()?) as f64)),
            0xcb => Ok(Value::from(f64::from_bits(self.read_u64()?))),
            0xe0..=0xff => Ok(Value::from((marker as i8) as i64)),
            _ => Err(MsgpackValueError::parse("unsupported MsgPack marker")),
        }
    }

    fn parse_string(&mut self, len: usize) -> Result<Value, MsgpackValueError> {
        let bytes = self.read_bytes(len)?;
        let text = std::str::from_utf8(bytes)
            .map_err(|_| MsgpackValueError::parse("invalid UTF-8 string in MsgPack"))?;
        Ok(Value::from(self.ctx.intern(text)))
    }

    fn parse_array(&mut self, len: usize) -> Result<Value, MsgpackValueError> {
        let array = self.ctx.new_array();
        let mut array_ref = array.borrow_mut();
        for _ in 0..len {
            array_ref.push(self.parse_value()?);
        }
        drop(array_ref);
        Ok(Value::from(array))
    }

    fn parse_map(&mut self, len: usize) -> Result<Value, MsgpackValueError> {
        let object = self.ctx.new_object();
        let mut object_ref = object.borrow_mut();
        for _ in 0..len {
            let atom = self.parse_key_atom()?;
            let value = self.parse_value()?;
            object_ref.set(atom, value);
        }
        drop(object_ref);
        Ok(Value::from(object))
    }

    fn parse_key_atom(&mut self) -> Result<Atom, MsgpackValueError> {
        let marker = self.read_u8()?;
        match marker {
            0xa0..=0xbf => self.parse_atom_string((marker & 0x1f) as usize),
            0xd9 => {
                let len = self.read_u8()? as usize;
                self.parse_atom_string(len)
            }
            0xda => {
                let len = self.read_u16()? as usize;
                self.parse_atom_string(len)
            }
            0xdb => {
                let len = self.read_u32()? as usize;
                self.parse_atom_string(len)
            }
            _ => Err(MsgpackValueError::parse("MsgPack map keys must be strings")),
        }
    }

    fn read_u8(&mut self) -> Result<u8, MsgpackValueError> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| MsgpackValueError::parse("unexpected end of MsgPack data"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_u16(&mut self) -> Result<u16, MsgpackValueError> {
        Ok(u16::from_be_bytes(self.read_array::<2>()?))
    }

    fn read_u32(&mut self) -> Result<u32, MsgpackValueError> {
        Ok(u32::from_be_bytes(self.read_array::<4>()?))
    }

    fn read_u64(&mut self) -> Result<u64, MsgpackValueError> {
        Ok(u64::from_be_bytes(self.read_array::<8>()?))
    }

    fn read_i8(&mut self) -> Result<i8, MsgpackValueError> {
        Ok(self.read_u8()? as i8)
    }

    fn read_i16(&mut self) -> Result<i16, MsgpackValueError> {
        Ok(i16::from_be_bytes(self.read_array::<2>()?))
    }

    fn read_i32(&mut self) -> Result<i32, MsgpackValueError> {
        Ok(i32::from_be_bytes(self.read_array::<4>()?))
    }

    fn read_i64(&mut self) -> Result<i64, MsgpackValueError> {
        Ok(i64::from_be_bytes(self.read_array::<8>()?))
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], MsgpackValueError> {
        let bytes = self.read_bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], MsgpackValueError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| MsgpackValueError::parse("invalid MsgPack length"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| MsgpackValueError::parse("unexpected end of MsgPack data"))?;
        self.pos = end;
        Ok(slice)
    }

    fn parse_atom_string(&mut self, len: usize) -> Result<Atom, MsgpackValueError> {
        let bytes = self.read_bytes(len)?;
        let text = std::str::from_utf8(bytes)
            .map_err(|_| MsgpackValueError::parse("invalid UTF-8 string in MsgPack"))?;
        Ok(self.ctx.intern(text))
    }
}
