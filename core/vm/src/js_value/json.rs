use std::collections::HashSet;

use crate::atoms::Atom;
use crate::gc::Gc;
use crate::heap::{
    QArray, QBoolArray, QFloat64Array, QInt32Array, QObject, QString, QStringArray, QUint8Array,
};
use crate::runtime::Context;

use super::{HeapKind, JsonValueError, Value};

pub(super) fn from_json(ctx: &Context, json: &str) -> Result<Value, JsonValueError> {
    JsonParser::new(ctx, json).parse()
}

pub(super) fn to_json(ctx: &Context, value: Value) -> Result<String, JsonValueError> {
    let mut out = String::with_capacity(256);
    let mut state = JsonWriteState::new(false);
    write_json_value(&mut out, ctx, value, &mut state)?;
    Ok(out)
}

pub(super) fn to_pretty_json(ctx: &Context, value: Value) -> Result<String, JsonValueError> {
    let mut out = String::with_capacity(256);
    let mut state = JsonWriteState::new(true);
    write_json_value(&mut out, ctx, value, &mut state)?;
    Ok(out)
}

struct JsonWriteState {
    pretty: bool,
    depth: usize,
    seen: HashSet<usize>,
}

impl JsonWriteState {
    fn new(pretty: bool) -> Self {
        Self {
            pretty,
            depth: 0,
            seen: HashSet::new(),
        }
    }
}

fn write_json_value(
    out: &mut String,
    ctx: &Context,
    value: Value,
    state: &mut JsonWriteState,
) -> Result<(), JsonValueError> {
    if value.is_null() {
        out.push_str("null");
        return Ok(());
    }

    if value.is_undefined() {
        return Err(JsonValueError::unsupported("undefined"));
    }

    if let Some(value) = value.as_bool() {
        out.push_str(if value { "true" } else { "false" });
        return Ok(());
    }

    if let Some(value) = value.as_i32() {
        let mut buf = itoa::Buffer::new();
        out.push_str(buf.format(value));
        return Ok(());
    }

    if let Some(value) = value.as_f64() {
        if !value.is_finite() {
            return Err(JsonValueError::invalid_number(value));
        }

        let mut buf = ryu::Buffer::new();
        out.push_str(buf.format(value));
        return Ok(());
    }

    if let Some(atom) = value.as_atom() {
        ctx.with_resolved(atom, |text| write_json_string(out, text));
        return Ok(());
    }

    match value.heap_kind() {
        Some(HeapKind::Object) => {
            write_json_object(out, ctx, Gc::<QObject>::try_from(value).unwrap(), state)
        }
        Some(HeapKind::Array) => {
            write_json_array(out, ctx, Gc::<QArray>::try_from(value).unwrap(), state)
        }
        Some(HeapKind::BoolArray) => {
            let array = Gc::<QBoolArray>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_json_bool_array(out, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::Uint8Array) => {
            let array = Gc::<QUint8Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_json_u8_array(out, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::Int32Array) => {
            let array = Gc::<QInt32Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_json_i32_array(out, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::Float64Array) => {
            let array = Gc::<QFloat64Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_json_f64_array(out, &array_ref.elements, state)
        }
        Some(HeapKind::StringArray) => {
            let array = Gc::<QStringArray>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_json_string_array(out, ctx, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::String) => {
            let string = Gc::<QString>::try_from(value).unwrap();
            let atom = string.borrow().atom;
            ctx.with_resolved(atom, |text| write_json_string(out, text));
            Ok(())
        }
        Some(kind) => Err(JsonValueError::unsupported(kind.type_name())),
        None => Err(JsonValueError::unsupported("unknown value")),
    }
}

fn write_json_object(
    out: &mut String,
    ctx: &Context,
    object: Gc<QObject>,
    state: &mut JsonWriteState,
) -> Result<(), JsonValueError> {
    let ptr = enter_heap(Value::from(&object), "object", state)?;
    let result = (|| {
        let object_ref = object.borrow();
        out.push('{');

        if !object_ref.shape.props.is_empty() {
            state.depth += 1;
            let mut first = true;
            for (&atom, &index) in &object_ref.shape.props {
                if first {
                    first = false;
                } else {
                    out.push(',');
                }

                if state.pretty {
                    out.push('\n');
                    write_indent(out, state.depth);
                }

                ctx.with_resolved(atom, |text| write_json_string(out, text));
                out.push(':');
                if state.pretty {
                    out.push(' ');
                }

                let value = object_ref
                    .values
                    .get(index)
                    .copied()
                    .unwrap_or_else(Value::undefined);
                write_json_value(out, ctx, value, state)?;
            }
            state.depth -= 1;

            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
        }

        out.push('}');
        Ok(())
    })();
    leave_heap(ptr, state);
    result
}

fn write_json_array(
    out: &mut String,
    ctx: &Context,
    array: Gc<QArray>,
    state: &mut JsonWriteState,
) -> Result<(), JsonValueError> {
    let ptr = enter_heap(Value::from(&array), "array", state)?;
    let result = (|| {
        let array_ref = array.borrow();
        out.push('[');

        if !array_ref.elements.is_empty() {
            state.depth += 1;
            for (index, &value) in array_ref.elements.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                if state.pretty {
                    out.push('\n');
                    write_indent(out, state.depth);
                }
                write_json_value(out, ctx, value, state)?;
            }
            state.depth -= 1;
            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
        }

        out.push(']');
        Ok(())
    })();
    leave_heap(ptr, state);
    result
}

#[inline]
fn write_json_bool_array(out: &mut String, values: &[bool], state: &mut JsonWriteState) {
    out.push('[');
    if !values.is_empty() {
        state.depth += 1;
        for (index, &value) in values.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
            out.push_str(if value { "true" } else { "false" });
        }
        state.depth -= 1;
        if state.pretty {
            out.push('\n');
            write_indent(out, state.depth);
        }
    }
    out.push(']');
}

#[inline]
fn write_json_u8_array(out: &mut String, values: &[u8], state: &mut JsonWriteState) {
    out.push('[');
    if !values.is_empty() {
        state.depth += 1;
        for (index, &value) in values.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
            let mut buf = itoa::Buffer::new();
            out.push_str(buf.format(value));
        }
        state.depth -= 1;
        if state.pretty {
            out.push('\n');
            write_indent(out, state.depth);
        }
    }
    out.push(']');
}

#[inline]
fn write_json_i32_array(out: &mut String, values: &[i32], state: &mut JsonWriteState) {
    out.push('[');
    if !values.is_empty() {
        state.depth += 1;
        for (index, &value) in values.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
            let mut buf = itoa::Buffer::new();
            out.push_str(buf.format(value));
        }
        state.depth -= 1;
        if state.pretty {
            out.push('\n');
            write_indent(out, state.depth);
        }
    }
    out.push(']');
}

#[inline]
fn write_json_f64_array(
    out: &mut String,
    values: &[f64],
    state: &mut JsonWriteState,
) -> Result<(), JsonValueError> {
    out.push('[');
    if !values.is_empty() {
        state.depth += 1;
        for (index, &value) in values.iter().enumerate() {
            if !value.is_finite() {
                return Err(JsonValueError::invalid_number(value));
            }
            if index != 0 {
                out.push(',');
            }
            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
            let mut buf = ryu::Buffer::new();
            out.push_str(buf.format(value));
        }
        state.depth -= 1;
        if state.pretty {
            out.push('\n');
            write_indent(out, state.depth);
        }
    }
    out.push(']');
    Ok(())
}

#[inline]
fn write_json_string_array(
    out: &mut String,
    ctx: &Context,
    values: &[Value],
    state: &mut JsonWriteState,
) {
    out.push('[');
    if !values.is_empty() {
        state.depth += 1;
        for (index, value) in values.iter().enumerate() {
            if index != 0 {
                out.push(',');
            }
            if state.pretty {
                out.push('\n');
                write_indent(out, state.depth);
            }
            write_json_string_like(out, ctx, *value);
        }
        state.depth -= 1;
        if state.pretty {
            out.push('\n');
            write_indent(out, state.depth);
        }
    }
    out.push(']');
}

#[inline]
fn write_json_string_like(out: &mut String, ctx: &Context, value: Value) {
    if let Some(atom) = value.as_atom() {
        ctx.with_resolved(atom, |text| write_json_string(out, text));
    } else if value.heap_kind() == Some(HeapKind::String) {
        let string = Gc::<QString>::try_from(value).unwrap();
        let atom = string.borrow().atom;
        ctx.with_resolved(atom, |text| write_json_string(out, text));
    } else {
        write_json_string(out, "");
    }
}

#[inline]
fn enter_heap(
    value: Value,
    type_name: &'static str,
    state: &mut JsonWriteState,
) -> Result<usize, JsonValueError> {
    let ptr = value
        .as_heap_ptr()
        .map(|ptr| ptr as usize)
        .ok_or_else(|| JsonValueError::unsupported(type_name))?;

    if !state.seen.insert(ptr) {
        return Err(JsonValueError::cyclic(type_name));
    }

    Ok(ptr)
}

#[inline(always)]
fn leave_heap(ptr: usize, state: &mut JsonWriteState) {
    state.seen.remove(&ptr);
}

#[inline(always)]
fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

#[inline]
fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1F}' => {
                out.push_str("\\u00");
                out.push(hex_digit(((c as u32) >> 4) as u8));
                out.push(hex_digit((c as u32) as u8));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn hex_digit(value: u8) -> char {
    match value & 0x0f {
        0..=9 => (b'0' + (value & 0x0f)) as char,
        _ => (b'a' + ((value & 0x0f) - 10)) as char,
    }
}

struct JsonParser<'a> {
    ctx: &'a Context,
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(ctx: &'a Context, input: &'a str) -> Self {
        Self {
            ctx,
            input,
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<Value, JsonValueError> {
        self.skip_ws();
        let value = self.parse_value()?;
        self.skip_ws();
        if self.pos != self.bytes.len() {
            return Err(self.error("trailing characters after JSON value"));
        }
        Ok(value)
    }

    #[inline]
    fn parse_value(&mut self) -> Result<Value, JsonValueError> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string_value(),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(b't') => self.parse_literal(b"true", Value::bool(true)),
            Some(b'f') => self.parse_literal(b"false", Value::bool(false)),
            Some(b'n') => self.parse_literal(b"null", Value::null()),
            Some(_) => Err(self.error("unexpected character in JSON value")),
            None => Err(self.error("unexpected end of JSON input")),
        }
    }

    fn parse_object(&mut self) -> Result<Value, JsonValueError> {
        self.expect_byte(b'{')?;
        let object = self.ctx.new_object();
        self.skip_ws();

        if self.consume_byte(b'}') {
            return Ok(Value::from(object));
        }

        let mut object_ref = object.borrow_mut();
        loop {
            self.skip_ws();
            let key = self.parse_string_atom()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            let value = self.parse_value()?;
            object_ref.set(key, value);
            self.skip_ws();

            if self.consume_byte(b'}') {
                break;
            }

            self.expect_byte(b',')?;
        }

        drop(object_ref);
        Ok(Value::from(object))
    }

    fn parse_array(&mut self) -> Result<Value, JsonValueError> {
        self.expect_byte(b'[')?;
        let array = self.ctx.new_array();
        self.skip_ws();

        if self.consume_byte(b']') {
            return Ok(Value::from(array));
        }

        let mut array_ref = array.borrow_mut();
        loop {
            let value = self.parse_value()?;
            array_ref.push(value);
            self.skip_ws();

            if self.consume_byte(b']') {
                break;
            }

            self.expect_byte(b',')?;
        }

        drop(array_ref);
        Ok(Value::from(array))
    }

    #[inline]
    fn parse_string_value(&mut self) -> Result<Value, JsonValueError> {
        Ok(Value::from(self.parse_string_atom()?))
    }

    #[inline]
    fn parse_string_atom(&mut self) -> Result<Atom, JsonValueError> {
        match self.parse_string()? {
            ParsedJsonString::Borrowed(value) => Ok(self.ctx.intern(value)),
            ParsedJsonString::Owned(value) => Ok(self.ctx.intern(&value)),
        }
    }

    fn parse_string(&mut self) -> Result<ParsedJsonString<'a>, JsonValueError> {
        self.expect_byte(b'"')?;
        let start = self.pos;

        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    let end = self.pos;
                    self.pos += 1;
                    return Ok(ParsedJsonString::Borrowed(&self.input[start..end]));
                }
                b'\\' => return self.parse_escaped_string(start),
                0x00..=0x1f => return Err(self.error("control character in JSON string")),
                _ => self.pos += 1,
            }
        }

        Err(self.error("unterminated JSON string"))
    }

    fn parse_escaped_string(
        &mut self,
        start: usize,
    ) -> Result<ParsedJsonString<'a>, JsonValueError> {
        // Pre-allocate with larger capacity since we know there's at least one escape
        let estimated_len = (self.bytes.len() - start) / 2 + 32;
        let mut out = String::with_capacity(estimated_len.min(4096));
        out.push_str(&self.input[start..self.pos]);

        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(ParsedJsonString::Owned(out));
                }
                b'\\' => {
                    self.pos += 1;
                    let escaped = self
                        .next_byte()
                        .ok_or_else(|| self.error("unterminated escape sequence"))?;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.parse_unicode_escape()?),
                        _ => return Err(self.error("invalid escape sequence")),
                    }
                }
                0x00..=0x1f => return Err(self.error("control character in JSON string")),
                _ => {
                    let chunk_start = self.pos;
                    self.pos += 1;
                    while let Some(next) = self.peek() {
                        match next {
                            b'"' | b'\\' | 0x00..=0x1f => break,
                            _ => self.pos += 1,
                        }
                    }
                    out.push_str(&self.input[chunk_start..self.pos]);
                }
            }
        }

        Err(self.error("unterminated JSON string"))
    }

    fn parse_unicode_escape(&mut self) -> Result<char, JsonValueError> {
        let first = self.parse_hex_u16()?;
        if !(0xd800..=0xdbff).contains(&first) {
            return char::from_u32(first as u32)
                .ok_or_else(|| self.error("invalid unicode escape"));
        }

        if self.next_byte() != Some(b'\\') || self.next_byte() != Some(b'u') {
            return Err(self.error("missing low surrogate in unicode escape"));
        }

        let second = self.parse_hex_u16()?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err(self.error("invalid low surrogate in unicode escape"));
        }

        let high = (first as u32) - 0xd800;
        let low = (second as u32) - 0xdc00;
        char::from_u32(0x10000 + ((high << 10) | low))
            .ok_or_else(|| self.error("invalid unicode surrogate pair"))
    }
    #[inline(always)]
    fn parse_hex_u16(&mut self) -> Result<u16, JsonValueError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let byte = self
                .next_byte()
                .ok_or_else(|| self.error("unexpected end of unicode escape"))?;
            value = (value << 4)
                | match byte {
                    b'0'..=b'9' => (byte - b'0') as u16,
                    b'a'..=b'f' => (byte - b'a' + 10) as u16,
                    b'A'..=b'F' => (byte - b'A' + 10) as u16,
                    _ => return Err(self.error("invalid hex digit in unicode escape")),
                };
        }
        Ok(value)
    }

    #[inline]
    fn parse_number(&mut self) -> Result<Value, JsonValueError> {
        let start = self.pos;

        self.consume_byte(b'-');

        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("invalid JSON number")),
        }

        let mut is_float = false;
        if self.consume_byte(b'.') {
            is_float = true;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid JSON number"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid JSON number exponent"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }

        let number = &self.input[start..self.pos];
        if is_float {
            return number
                .parse::<f64>()
                .map(Value::from)
                .map_err(|_| self.error("invalid JSON number"));
        }

        if number.starts_with('-') {
            number
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| self.error("invalid JSON number"))
        } else {
            number
                .parse::<u64>()
                .map(Value::from)
                .map_err(|_| self.error("invalid JSON number"))
        }
    }

    #[inline]
    fn parse_literal(&mut self, literal: &[u8], value: Value) -> Result<Value, JsonValueError> {
        if self.bytes.get(self.pos..self.pos + literal.len()) == Some(literal) {
            self.pos += literal.len();
            Ok(value)
        } else {
            Err(self.error("invalid JSON literal"))
        }
    }

    #[inline(always)]
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    #[inline]
    fn expect_byte(&mut self, byte: u8) -> Result<(), JsonValueError> {
        if self.consume_byte(byte) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{}'", byte as char)))
        }
    }

    #[inline(always)]
    fn consume_byte(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline(always)]
    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn error(&self, message: impl Into<String>) -> JsonValueError {
        JsonValueError::parse(format!("{} at byte {}", message.into(), self.pos))
    }
}

enum ParsedJsonString<'a> {
    Borrowed(&'a str),
    Owned(String),
}
