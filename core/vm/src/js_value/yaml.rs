use crate::gc::Gc;
use crate::heap::{
    QArray, QBoolArray, QFloat64Array, QInt32Array, QObject, QString, QStringArray, QUint8Array,
};
use crate::runtime::Context;

use super::{HeapKind, Value, YamlValueError};

pub(super) fn from_yaml(ctx: &Context, yaml: &str) -> Result<Value, YamlValueError> {
    YamlParser::new(ctx, yaml).parse()
}

pub(super) fn to_yaml(ctx: &Context, value: Value) -> Result<String, YamlValueError> {
    let mut out = String::with_capacity(256);
    let mut state = YamlWriteState::new();
    write_yaml_node(&mut out, ctx, value, &mut state)?;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

struct YamlWriteState {
    depth: usize,
    seen: Vec<usize>,
}

impl YamlWriteState {
    fn new() -> Self {
        Self {
            depth: 0,
            seen: Vec::with_capacity(32),
        }
    }
}

fn write_yaml_node(
    out: &mut String,
    ctx: &Context,
    value: Value,
    state: &mut YamlWriteState,
) -> Result<(), YamlValueError> {
    if is_inline_yaml_value(value) {
        write_yaml_inline(out, ctx, value)?;
        return Ok(());
    }

    match value.heap_kind() {
        Some(HeapKind::Object) => {
            write_yaml_object(out, ctx, Gc::<QObject>::try_from(value).unwrap(), state)
        }
        Some(HeapKind::Array) => {
            write_yaml_array(out, ctx, Gc::<QArray>::try_from(value).unwrap(), state)
        }
        Some(HeapKind::BoolArray) => {
            let array = Gc::<QBoolArray>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_yaml_bool_array(out, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::Uint8Array) => {
            let array = Gc::<QUint8Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_yaml_u8_array(out, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::Int32Array) => {
            let array = Gc::<QInt32Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_yaml_i32_array(out, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::Float64Array) => {
            let array = Gc::<QFloat64Array>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_yaml_f64_array(out, &array_ref.elements, state)
        }
        Some(HeapKind::StringArray) => {
            let array = Gc::<QStringArray>::try_from(value).unwrap();
            let array_ref = array.borrow();
            write_yaml_string_array(out, ctx, &array_ref.elements, state);
            Ok(())
        }
        Some(HeapKind::String) => {
            let string = Gc::<QString>::try_from(value).unwrap();
            let atom = string.borrow().atom;
            ctx.with_resolved(atom, |text| write_yaml_string(out, text));
            Ok(())
        }
        Some(kind) => Err(YamlValueError::unsupported(kind.type_name())),
        None => Err(YamlValueError::unsupported("unknown value")),
    }
}

fn is_inline_yaml_value(value: Value) -> bool {
    value.is_null()
        || value.is_undefined()
        || value.as_bool().is_some()
        || value.as_i32().is_some()
        || value.as_f64().is_some()
        || value.as_atom().is_some()
        || value.heap_kind() == Some(HeapKind::String)
}

fn write_yaml_inline(out: &mut String, ctx: &Context, value: Value) -> Result<(), YamlValueError> {
    if value.is_null() {
        out.push_str("null");
        return Ok(());
    }

    if value.is_undefined() {
        return Err(YamlValueError::unsupported("undefined"));
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
            return Err(YamlValueError::invalid_number(value));
        }
        let mut buf = ryu::Buffer::new();
        out.push_str(buf.format(value));
        return Ok(());
    }

    if let Some(atom) = value.as_atom() {
        ctx.with_resolved(atom, |text| write_yaml_string(out, text));
        return Ok(());
    }

    if value.heap_kind() == Some(HeapKind::String) {
        let string = Gc::<QString>::try_from(value).unwrap();
        let atom = string.borrow().atom;
        ctx.with_resolved(atom, |text| write_yaml_string(out, text));
        return Ok(());
    }

    Err(YamlValueError::unsupported(value.type_name()))
}

fn write_yaml_object(
    out: &mut String,
    ctx: &Context,
    object: Gc<QObject>,
    state: &mut YamlWriteState,
) -> Result<(), YamlValueError> {
    let ptr = enter_heap(Value::from(&object), "object", state)?;
    let result = (|| {
        let object_ref = object.borrow();
        if object_ref.shape.props.is_empty() {
            out.push_str("{}");
            return Ok(());
        }

        let mut first = true;
        for (&atom, &index) in &object_ref.shape.props {
            if !first {
                out.push('\n');
            }
            first = false;

            write_indent(out, state.depth);
            ctx.with_resolved(atom, |text| write_yaml_string(out, text));
            out.push(':');

            let value = object_ref
                .values
                .get(index)
                .copied()
                .unwrap_or_else(Value::undefined);

            if is_inline_yaml_value(value) {
                out.push(' ');
                write_yaml_inline(out, ctx, value)?;
            } else {
                out.push('\n');
                state.depth += 1;
                write_yaml_node(out, ctx, value, state)?;
                state.depth -= 1;
            }
        }

        Ok(())
    })();
    leave_heap(ptr, state);
    result
}

fn write_yaml_array(
    out: &mut String,
    ctx: &Context,
    array: Gc<QArray>,
    state: &mut YamlWriteState,
) -> Result<(), YamlValueError> {
    let ptr = enter_heap(Value::from(&array), "array", state)?;
    let result = (|| {
        let array_ref = array.borrow();
        if array_ref.elements.is_empty() {
            out.push_str("[]");
            return Ok(());
        }

        for (index, &value) in array_ref.elements.iter().enumerate() {
            if index != 0 {
                out.push('\n');
            }

            write_indent(out, state.depth);
            out.push('-');

            if is_inline_yaml_value(value) {
                out.push(' ');
                write_yaml_inline(out, ctx, value)?;
            } else {
                out.push('\n');
                state.depth += 1;
                write_yaml_node(out, ctx, value, state)?;
                state.depth -= 1;
            }
        }

        Ok(())
    })();
    leave_heap(ptr, state);
    result
}

fn write_yaml_bool_array(out: &mut String, values: &[bool], state: &mut YamlWriteState) {
    if values.is_empty() {
        out.push_str("[]");
        return;
    }

    for (index, &value) in values.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        write_indent(out, state.depth);
        out.push_str("- ");
        out.push_str(if value { "true" } else { "false" });
    }
}

fn write_yaml_u8_array(out: &mut String, values: &[u8], state: &mut YamlWriteState) {
    if values.is_empty() {
        out.push_str("[]");
        return;
    }

    for (index, &value) in values.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        write_indent(out, state.depth);
        out.push_str("- ");
        let mut buf = itoa::Buffer::new();
        out.push_str(buf.format(value));
    }
}

fn write_yaml_i32_array(out: &mut String, values: &[i32], state: &mut YamlWriteState) {
    if values.is_empty() {
        out.push_str("[]");
        return;
    }

    for (index, &value) in values.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        write_indent(out, state.depth);
        out.push_str("- ");
        let mut buf = itoa::Buffer::new();
        out.push_str(buf.format(value));
    }
}

fn write_yaml_f64_array(
    out: &mut String,
    values: &[f64],
    state: &mut YamlWriteState,
) -> Result<(), YamlValueError> {
    if values.is_empty() {
        out.push_str("[]");
        return Ok(());
    }

    for (index, &value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(YamlValueError::invalid_number(value));
        }
        if index != 0 {
            out.push('\n');
        }
        write_indent(out, state.depth);
        out.push_str("- ");
        let mut buf = ryu::Buffer::new();
        out.push_str(buf.format(value));
    }

    Ok(())
}

fn write_yaml_string_array(
    out: &mut String,
    ctx: &Context,
    values: &[Value],
    state: &mut YamlWriteState,
) {
    if values.is_empty() {
        out.push_str("[]");
        return;
    }

    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            out.push('\n');
        }
        write_indent(out, state.depth);
        out.push_str("- ");
        write_yaml_string_like(out, ctx, *value);
    }
}

#[inline]
fn write_yaml_string_like(out: &mut String, ctx: &Context, value: Value) {
    if let Some(atom) = value.as_atom() {
        ctx.with_resolved(atom, |text| write_yaml_string(out, text));
    } else if value.heap_kind() == Some(HeapKind::String) {
        let string = Gc::<QString>::try_from(value).unwrap();
        let atom = string.borrow().atom;
        ctx.with_resolved(atom, |text| write_yaml_string(out, text));
    } else {
        write_yaml_string(out, "");
    }
}

fn enter_heap(
    value: Value,
    type_name: &'static str,
    state: &mut YamlWriteState,
) -> Result<usize, YamlValueError> {
    let ptr = value
        .as_heap_ptr()
        .map(|ptr| ptr as usize)
        .ok_or_else(|| YamlValueError::unsupported(type_name))?;

    if state.seen.contains(&ptr) {
        return Err(YamlValueError::cyclic(type_name));
    }

    state.seen.push(ptr);

    Ok(ptr)
}

fn leave_heap(ptr: usize, state: &mut YamlWriteState) {
    debug_assert_eq!(state.seen.last().copied(), Some(ptr));
    state.seen.pop();
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_yaml_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
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

struct YamlParser<'a> {
    ctx: &'a Context,
    lines: Vec<YamlLine<'a>>,
    index: usize,
}

impl<'a> YamlParser<'a> {
    fn new(ctx: &'a Context, input: &'a str) -> Self {
        let lines = input
            .lines()
            .filter_map(|line| {
                let trimmed_end = line.trim_end();
                if trimmed_end.trim().is_empty() {
                    return None;
                }
                let indent = trimmed_end.len() - trimmed_end.trim_start().len();
                Some(YamlLine {
                    indent,
                    text: trimmed_end.trim_start(),
                })
            })
            .collect();

        Self {
            ctx,
            lines,
            index: 0,
        }
    }

    fn parse(mut self) -> Result<Value, YamlValueError> {
        if self.lines.is_empty() {
            return Err(YamlValueError::parse("empty YAML input"));
        }

        let value = self.parse_node(0)?;
        if self.index != self.lines.len() {
            return Err(YamlValueError::parse("trailing YAML content"));
        }
        Ok(value)
    }

    fn parse_node(&mut self, indent: usize) -> Result<Value, YamlValueError> {
        let line = self
            .current()
            .ok_or_else(|| YamlValueError::parse("unexpected end of YAML input"))?;

        if line.indent < indent {
            return Err(YamlValueError::parse("invalid YAML indentation"));
        }

        if line.indent > indent {
            return Err(YamlValueError::parse("unexpected deeper indentation"));
        }

        if line.text.starts_with("-") {
            return self.parse_array(indent);
        }

        if line.text.starts_with('"') {
            if self.find_map_separator(line.text).is_some() {
                return self.parse_object(indent);
            }
            return self.parse_scalar(line.text);
        }

        self.parse_scalar(line.text)
    }

    fn parse_object(&mut self, indent: usize) -> Result<Value, YamlValueError> {
        let object = self.ctx.new_object();
        let mut object_ref = object.borrow_mut();

        while let Some(line) = self.current() {
            if line.indent != indent || !line.text.starts_with('"') {
                break;
            }

            let Some((key_text, rest)) = self.split_key_value(line.text) else {
                return Err(YamlValueError::parse("invalid YAML mapping entry"));
            };

            let key = match self.parse_string_literal(key_text)? {
                ParsedString::Borrowed(value) => self.ctx.intern(value),
                ParsedString::Owned(value) => self.ctx.intern(&value),
            };

            self.index += 1;

            let value = if rest.is_empty() {
                self.parse_node(indent + 2)?
            } else {
                self.parse_scalar(rest)?
            };

            object_ref.set(key, value);
        }

        drop(object_ref);
        Ok(Value::from(object))
    }

    fn parse_array(&mut self, indent: usize) -> Result<Value, YamlValueError> {
        let array = self.ctx.new_array();
        let mut array_ref = array.borrow_mut();

        while let Some(line) = self.current() {
            if line.indent != indent || !line.text.starts_with('-') {
                break;
            }

            let rest = line.text[1..].trim_start();
            self.index += 1;

            let value = if rest.is_empty() {
                self.parse_node(indent + 2)?
            } else {
                self.parse_scalar(rest)?
            };
            array_ref.push(value);
        }

        drop(array_ref);
        Ok(Value::from(array))
    }

    fn parse_scalar(&self, text: &str) -> Result<Value, YamlValueError> {
        match text {
            "null" => Ok(Value::null()),
            "true" => Ok(Value::bool(true)),
            "false" => Ok(Value::bool(false)),
            "[]" => {
                let array = self.ctx.new_array();
                Ok(Value::from(array))
            }
            "{}" => {
                let object = self.ctx.new_object();
                Ok(Value::from(object))
            }
            _ if text.starts_with('"') => {
                let atom = match self.parse_string_literal(text)? {
                    ParsedString::Borrowed(value) => self.ctx.intern(value),
                    ParsedString::Owned(value) => self.ctx.intern(&value),
                };
                Ok(Value::from(atom))
            }
            _ => {
                if text.contains(['.', 'e', 'E']) {
                    return text
                        .parse::<f64>()
                        .map(Value::from)
                        .map_err(|_| YamlValueError::parse("invalid YAML number"));
                }

                if text.starts_with('-') {
                    return text
                        .parse::<i64>()
                        .map(Value::from)
                        .map_err(|_| YamlValueError::parse("invalid YAML number"));
                }

                text.parse::<u64>()
                    .map(Value::from)
                    .map_err(|_| YamlValueError::parse("invalid YAML scalar"))
            }
        }
    }

    fn split_key_value<'b>(&self, text: &'b str) -> Option<(&'b str, &'b str)> {
        let colon = self.find_map_separator(text)?;
        let key = text[..colon].trim_end();
        let rest = text[colon + 1..].trim_start();
        Some((key, rest))
    }

    fn find_map_separator(&self, text: &str) -> Option<usize> {
        let mut escaped = false;
        for (idx, ch) in text.char_indices().skip(1) {
            match ch {
                '\\' => escaped = !escaped,
                '"' if !escaped => {
                    return text[idx + ch.len_utf8()..]
                        .find(':')
                        .map(|offset| idx + ch.len_utf8() + offset);
                }
                _ => escaped = false,
            }
        }
        None
    }

    fn parse_string_literal<'b>(&self, text: &'b str) -> Result<ParsedString<'b>, YamlValueError> {
        if !text.starts_with('"') || !text.ends_with('"') {
            return Err(YamlValueError::parse("invalid YAML string"));
        }

        let inner = &text[1..text.len() - 1];
        if !inner.contains('\\') {
            return Ok(ParsedString::Borrowed(inner));
        }

        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }

            let escaped = chars
                .next()
                .ok_or_else(|| YamlValueError::parse("unterminated YAML escape"))?;
            match escaped {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let mut hex = String::with_capacity(4);
                    for _ in 0..4 {
                        hex.push(
                            chars.next().ok_or_else(|| {
                                YamlValueError::parse("invalid YAML unicode escape")
                            })?,
                        );
                    }
                    let value = u16::from_str_radix(&hex, 16)
                        .map_err(|_| YamlValueError::parse("invalid YAML unicode escape"))?;
                    let ch = char::from_u32(value as u32)
                        .ok_or_else(|| YamlValueError::parse("invalid YAML unicode scalar"))?;
                    out.push(ch);
                }
                _ => return Err(YamlValueError::parse("invalid YAML escape")),
            }
        }

        Ok(ParsedString::Owned(out))
    }

    fn current(&self) -> Option<YamlLine<'a>> {
        self.lines.get(self.index).copied()
    }
}

#[derive(Clone, Copy)]
struct YamlLine<'a> {
    indent: usize,
    text: &'a str,
}

enum ParsedString<'a> {
    Borrowed(&'a str),
    Owned(String),
}
