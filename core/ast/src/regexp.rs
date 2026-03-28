use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::OnceLock,
};

use unicode_ident::{is_xid_continue, is_xid_start};

use crate::regexp_property_data::{
    PROPERTY_OF_STRINGS_EXPRESSIONS, VALID_CHARACTER_PROPERTY_EXPRESSIONS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegExpError {
    pub message: String,
}

impl RegExpError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RegExpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegExpFlags {
    pub source: String,
    pub has_indices: bool,
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub unicode_sets: bool,
    pub sticky: bool,
}

impl RegExpFlags {
    pub const fn unicode_mode(&self) -> bool {
        self.unicode || self.unicode_sets
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegExpPattern {
    pub source: String,
    pub flags: RegExpFlags,
    pub disjunction: RegExpDisjunction,
    pub capture_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegExpDisjunction {
    pub alternatives: Vec<RegExpAlternative>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegExpAlternative {
    pub terms: Vec<RegExpTerm>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegExpTerm {
    Assertion(RegExpAssertion),
    Atom {
        atom: RegExpAtom,
        quantifier: Option<RegExpQuantifier>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegExpAssertion {
    Start,
    End,
    WordBoundary,
    NonWordBoundary,
    LookAhead {
        negative: bool,
        disjunction: RegExpDisjunction,
    },
    LookBehind {
        negative: bool,
        disjunction: RegExpDisjunction,
    },
}

impl RegExpAssertion {
    const fn is_legacy_quantifiable(&self) -> bool {
        matches!(self, Self::LookAhead { .. })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegExpAtom {
    Raw(String),
    Group(RegExpGroup),
    NamedBackreference { name: String },
    LegacyNamedEscape { raw: String, name: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegExpGroup {
    pub kind: RegExpGroupKind,
    pub capture_index: Option<usize>,
    pub disjunction: RegExpDisjunction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegExpGroupKind {
    Capture { name: Option<String> },
    NonCapture,
    Modifiers { add: String, remove: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegExpQuantifier {
    pub min: u32,
    pub max: Option<u32>,
    pub greedy: bool,
}

#[derive(Debug, Clone)]
struct NamedGroupOccurrence<'a> {
    name: &'a str,
    path: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
struct PendingNamedBackreference<'a> {
    name: Option<&'a str>,
    legacy_fallback: bool,
}

pub fn parse_regexp_pattern(body: &str, flags: &str) -> Result<RegExpPattern, RegExpError> {
    let flags = parse_flags(flags)?;
    let (disjunction, capture_count) = {
        let mut parser = Parser::new(body, &flags);
        let disjunction = parser.parse_disjunction(None)?;
        if !parser.is_eof() {
            return Err(RegExpError::new("extraneous characters at the end"));
        }
        (disjunction, parser.capture_count)
    };

    let pattern = RegExpPattern {
        source: body.to_string(),
        flags,
        disjunction,
        capture_count,
    };

    validate_named_groups(&pattern)?;
    validate_unicode_sets_early_errors(&pattern)?;
    validate_unicode_mode_raw_semantics(&pattern)?;
    Ok(pattern)
}

fn parse_flags(flags: &str) -> Result<RegExpFlags, RegExpError> {
    let mut result = RegExpFlags {
        source: flags.to_string(),
        has_indices: false,
        global: false,
        ignore_case: false,
        multiline: false,
        dot_all: false,
        unicode: false,
        unicode_sets: false,
        sticky: false,
    };

    for ch in flags.chars() {
        let slot = match ch {
            'd' => &mut result.has_indices,
            'g' => &mut result.global,
            'i' => &mut result.ignore_case,
            'm' => &mut result.multiline,
            's' => &mut result.dot_all,
            'u' => &mut result.unicode,
            'v' => &mut result.unicode_sets,
            'y' => &mut result.sticky,
            _ => return Err(RegExpError::new("invalid regular expression flags")),
        };
        if *slot {
            return Err(RegExpError::new("duplicate regular expression flag"));
        }
        *slot = true;
    }

    if result.unicode && result.unicode_sets {
        return Err(RegExpError::new(
            "regular expression cannot use both 'u' and 'v' flags",
        ));
    }

    Ok(result)
}

struct Parser<'a> {
    chars: Vec<char>,
    index: usize,
    flags: &'a RegExpFlags,
    capture_count: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &str, flags: &'a RegExpFlags) -> Self {
        Self {
            chars: source.chars().collect(),
            index: 0,
            flags,
            capture_count: 0,
        }
    }

    fn parse_disjunction(
        &mut self,
        terminator: Option<char>,
    ) -> Result<RegExpDisjunction, RegExpError> {
        let mut alternatives = vec![self.parse_alternative(terminator)?];
        while self.peek() == Some('|') {
            self.bump();
            alternatives.push(self.parse_alternative(terminator)?);
        }
        Ok(RegExpDisjunction { alternatives })
    }

    fn parse_alternative(
        &mut self,
        terminator: Option<char>,
    ) -> Result<RegExpAlternative, RegExpError> {
        let mut terms = Vec::new();
        while !self.is_eof() {
            if self.peek() == Some('|') || terminator.is_some_and(|ch| self.peek() == Some(ch)) {
                break;
            }
            let Some(term) = self.parse_term()? else {
                break;
            };
            terms.push(term);
        }
        Ok(RegExpAlternative { terms })
    }

    fn parse_term(&mut self) -> Result<Option<RegExpTerm>, RegExpError> {
        let Some(ch) = self.peek() else {
            return Ok(None);
        };
        if ch == ')' || ch == '|' {
            return Ok(None);
        }

        if let Some(assertion) = self.try_parse_assertion()? {
            let quantifier = self.try_parse_quantifier()?;
            if quantifier.is_some() {
                if !assertion.is_legacy_quantifiable() || self.flags.unicode_mode() {
                    return Err(RegExpError::new("nothing to repeat"));
                }
                return Ok(Some(RegExpTerm::Atom {
                    atom: RegExpAtom::Raw(emit_assertion_as_group(&assertion)),
                    quantifier,
                }));
            }
            return Ok(Some(RegExpTerm::Assertion(assertion)));
        }

        let atom = self.parse_atom()?;
        let quantifier = self.try_parse_quantifier()?;
        Ok(Some(RegExpTerm::Atom { atom, quantifier }))
    }

    fn try_parse_assertion(&mut self) -> Result<Option<RegExpAssertion>, RegExpError> {
        let Some(ch) = self.peek() else {
            return Ok(None);
        };
        match ch {
            '^' => {
                self.bump();
                Ok(Some(RegExpAssertion::Start))
            }
            '$' => {
                self.bump();
                Ok(Some(RegExpAssertion::End))
            }
            '\\' if self.peek_n(1) == Some('b') => {
                self.bump();
                self.bump();
                Ok(Some(RegExpAssertion::WordBoundary))
            }
            '\\' if self.peek_n(1) == Some('B') => {
                self.bump();
                self.bump();
                Ok(Some(RegExpAssertion::NonWordBoundary))
            }
            '(' if self.peek_n(1) == Some('?') && self.peek_n(2) == Some('=') => {
                self.bump();
                self.bump();
                self.bump();
                let disjunction = self.parse_disjunction(Some(')'))?;
                self.expect(')')?;
                Ok(Some(RegExpAssertion::LookAhead {
                    negative: false,
                    disjunction,
                }))
            }
            '(' if self.peek_n(1) == Some('?') && self.peek_n(2) == Some('!') => {
                self.bump();
                self.bump();
                self.bump();
                let disjunction = self.parse_disjunction(Some(')'))?;
                self.expect(')')?;
                Ok(Some(RegExpAssertion::LookAhead {
                    negative: true,
                    disjunction,
                }))
            }
            '(' if self.peek_n(1) == Some('?')
                && self.peek_n(2) == Some('<')
                && self.peek_n(3) == Some('=') =>
            {
                self.bump();
                self.bump();
                self.bump();
                self.bump();
                let disjunction = self.parse_disjunction(Some(')'))?;
                self.expect(')')?;
                Ok(Some(RegExpAssertion::LookBehind {
                    negative: false,
                    disjunction,
                }))
            }
            '(' if self.peek_n(1) == Some('?')
                && self.peek_n(2) == Some('<')
                && self.peek_n(3) == Some('!') =>
            {
                self.bump();
                self.bump();
                self.bump();
                self.bump();
                let disjunction = self.parse_disjunction(Some(')'))?;
                self.expect(')')?;
                Ok(Some(RegExpAssertion::LookBehind {
                    negative: true,
                    disjunction,
                }))
            }
            _ => Ok(None),
        }
    }

    fn parse_atom(&mut self) -> Result<RegExpAtom, RegExpError> {
        let Some(ch) = self.peek() else {
            return Err(RegExpError::new("unexpected end of regular expression"));
        };

        match ch {
            '(' => self.parse_group_atom(),
            '[' => Ok(RegExpAtom::Raw(self.scan_character_class()?)),
            '.' => {
                self.bump();
                Ok(RegExpAtom::Raw(".".to_string()))
            }
            '\\' => self.parse_escape_atom(),
            '*' | '+' | '?' => Err(RegExpError::new("nothing to repeat")),
            '{' => {
                if self.flags.unicode_mode() {
                    Err(RegExpError::new("syntax error"))
                } else if self.looks_like_quantifier_here() {
                    Err(RegExpError::new("nothing to repeat"))
                } else {
                    self.bump();
                    Ok(RegExpAtom::Raw("{".to_string()))
                }
            }
            ')' | '|' => Err(RegExpError::new("syntax error")),
            ']' if !self.flags.unicode_mode() => {
                self.bump();
                Ok(RegExpAtom::Raw("]".to_string()))
            }
            '}' if !self.flags.unicode_mode() => {
                self.bump();
                Ok(RegExpAtom::Raw("}".to_string()))
            }
            _ if is_regexp_syntax_character(ch) => Err(RegExpError::new("syntax error")),
            _ => {
                self.bump();
                Ok(RegExpAtom::Raw(ch.to_string()))
            }
        }
    }

    fn parse_group_atom(&mut self) -> Result<RegExpAtom, RegExpError> {
        self.expect('(')?;
        if self.peek() != Some('?') {
            let capture_index = self.next_capture_index();
            let disjunction = self.parse_disjunction(Some(')'))?;
            self.expect(')')?;
            return Ok(RegExpAtom::Group(RegExpGroup {
                kind: RegExpGroupKind::Capture { name: None },
                capture_index: Some(capture_index),
                disjunction,
            }));
        }

        self.expect('?')?;
        match self.peek() {
            Some(':') => {
                self.bump();
                let disjunction = self.parse_disjunction(Some(')'))?;
                self.expect(')')?;
                Ok(RegExpAtom::Group(RegExpGroup {
                    kind: RegExpGroupKind::NonCapture,
                    capture_index: None,
                    disjunction,
                }))
            }
            Some('<') if self.peek_n(1) != Some('=') && self.peek_n(1) != Some('!') => {
                self.bump();
                let name = self.parse_group_name()?;
                let capture_index = self.next_capture_index();
                let disjunction = self.parse_disjunction(Some(')'))?;
                self.expect(')')?;
                Ok(RegExpAtom::Group(RegExpGroup {
                    kind: RegExpGroupKind::Capture { name: Some(name) },
                    capture_index: Some(capture_index),
                    disjunction,
                }))
            }
            Some('i' | 'm' | 's' | '-') => self.parse_modifier_group(),
            _ => Err(RegExpError::new("invalid group")),
        }
    }

    fn parse_modifier_group(&mut self) -> Result<RegExpAtom, RegExpError> {
        let mut add = String::new();
        while matches!(self.peek(), Some('i' | 'm' | 's')) {
            add.push(self.bump().unwrap());
        }

        let mut remove = String::new();
        if self.peek() == Some('-') {
            self.bump();
            while matches!(self.peek(), Some('i' | 'm' | 's')) {
                remove.push(self.bump().unwrap());
            }
        }

        if self.peek() != Some(':') {
            return Err(RegExpError::new("invalid group"));
        }
        self.bump();

        if add.is_empty() && remove.is_empty() {
            return Err(RegExpError::new("regexp modifier group cannot be empty"));
        }
        if has_duplicate_modifier(&add) || has_duplicate_modifier(&remove) {
            return Err(RegExpError::new("duplicate regexp modifier"));
        }
        if add.chars().any(|ch| remove.contains(ch)) {
            return Err(RegExpError::new(
                "regexp modifier cannot be both added and removed",
            ));
        }

        let disjunction = self.parse_disjunction(Some(')'))?;
        self.expect(')')?;
        Ok(RegExpAtom::Group(RegExpGroup {
            kind: RegExpGroupKind::Modifiers { add, remove },
            capture_index: None,
            disjunction,
        }))
    }

    fn parse_escape_atom(&mut self) -> Result<RegExpAtom, RegExpError> {
        if self.peek_n(1) == Some('k') {
            if self.flags.unicode_mode() && self.peek_n(2) == Some('<') {
                self.expect('\\')?;
                self.expect('k')?;
                self.expect('<')?;
                let name = self.parse_group_name()?;
                return Ok(RegExpAtom::NamedBackreference { name });
            }
            if !self.flags.unicode_mode() && self.peek_n(2) == Some('<') {
                return self.parse_legacy_named_escape();
            }
            if !self.flags.unicode_mode() {
                self.expect('\\')?;
                self.expect('k')?;
                return Ok(RegExpAtom::LegacyNamedEscape {
                    raw: String::from("\\k"),
                    name: None,
                });
            }
        }
        Ok(RegExpAtom::Raw(self.scan_raw_escape()?))
    }

    fn parse_legacy_named_escape(&mut self) -> Result<RegExpAtom, RegExpError> {
        self.expect('\\')?;
        self.expect('k')?;
        self.expect('<')?;

        let saved = self.index;
        let parsed_name = self.parse_group_name().ok();
        let parsed_end = self.index;
        self.index = saved;

        let mut raw = String::from("\\k<");
        while let Some(ch) = self.peek() {
            if ch == '>' {
                raw.push(ch);
                self.bump();
                break;
            }
            if is_legacy_named_escape_terminator(ch) {
                break;
            }
            raw.push(ch);
            self.bump();
        }

        let name = if parsed_name.is_some() && self.index == parsed_end {
            parsed_name
        } else {
            None
        };

        Ok(RegExpAtom::LegacyNamedEscape { raw, name })
    }

    fn parse_group_name(&mut self) -> Result<String, RegExpError> {
        let first = self.parse_group_name_code_point(true)?;
        let mut value = String::new();
        value.push(first);
        while self.peek() != Some('>') {
            let ch = self.parse_group_name_code_point(false)?;
            value.push(ch);
        }
        self.expect('>')?;
        Ok(value)
    }

    fn parse_group_name_code_point(&mut self, is_start: bool) -> Result<char, RegExpError> {
        let Some(ch) = self.peek() else {
            return Err(RegExpError::new("invalid group name"));
        };
        if ch == '>' {
            return Err(RegExpError::new("invalid group name"));
        }

        let value = if ch == '\\' {
            self.parse_group_name_escape()?
        } else {
            self.bump().unwrap()
        };

        let valid = if is_start {
            is_identifier_start(value)
        } else {
            is_identifier_part(value)
        };
        if !valid {
            return Err(RegExpError::new("invalid group name"));
        }
        Ok(value)
    }

    fn parse_group_name_escape(&mut self) -> Result<char, RegExpError> {
        self.expect('\\')?;
        self.expect('u')?;
        if self.peek() == Some('{') {
            self.bump();
            let mut value = 0u32;
            let mut digits = 0usize;
            while let Some(ch) = self.peek() {
                if ch == '}' {
                    break;
                }
                let digit = ch
                    .to_digit(16)
                    .ok_or_else(|| RegExpError::new("invalid group name"))?;
                value = value
                    .checked_mul(16)
                    .and_then(|current| current.checked_add(digit))
                    .ok_or_else(|| RegExpError::new("invalid group name"))?;
                digits += 1;
                self.bump();
            }
            if digits == 0 || self.peek() != Some('}') {
                return Err(RegExpError::new("invalid group name"));
            }
            self.bump();
            return char::from_u32(value).ok_or_else(|| RegExpError::new("invalid group name"));
        }

        let first = self.read_exact_hex4()?;
        if (0xD800..=0xDBFF).contains(&first)
            && self.peek() == Some('\\')
            && self.peek_n(1) == Some('u')
        {
            let saved = self.index;
            self.bump();
            self.bump();
            let second = self.read_exact_hex4()?;
            if (0xDC00..=0xDFFF).contains(&second) {
                let cp = 0x10000 + (((first - 0xD800) << 10) | (second - 0xDC00));
                return char::from_u32(cp).ok_or_else(|| RegExpError::new("invalid group name"));
            }
            self.index = saved;
        }

        char::from_u32(first).ok_or_else(|| RegExpError::new("invalid group name"))
    }

    fn read_exact_hex4(&mut self) -> Result<u32, RegExpError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let ch = self
                .peek()
                .ok_or_else(|| RegExpError::new("invalid group name"))?;
            let digit = ch
                .to_digit(16)
                .ok_or_else(|| RegExpError::new("invalid group name"))?;
            value = value * 16 + digit;
            self.bump();
        }
        Ok(value)
    }

    fn try_parse_quantifier(&mut self) -> Result<Option<RegExpQuantifier>, RegExpError> {
        let Some(ch) = self.peek() else {
            return Ok(None);
        };
        let mut quantifier = match ch {
            '*' => {
                self.bump();
                RegExpQuantifier {
                    min: 0,
                    max: None,
                    greedy: true,
                }
            }
            '+' => {
                self.bump();
                RegExpQuantifier {
                    min: 1,
                    max: None,
                    greedy: true,
                }
            }
            '?' => {
                self.bump();
                RegExpQuantifier {
                    min: 0,
                    max: Some(1),
                    greedy: true,
                }
            }
            '{' if self.looks_like_quantifier_here() => self.parse_braced_quantifier()?,
            _ => return Ok(None),
        };

        if self.peek() == Some('?') {
            self.bump();
            quantifier.greedy = false;
        }
        Ok(Some(quantifier))
    }

    fn parse_braced_quantifier(&mut self) -> Result<RegExpQuantifier, RegExpError> {
        self.expect('{')?;
        let min = self.parse_decimal_digits()?;
        let max = if self.peek() == Some(',') {
            self.bump();
            if self.peek() == Some('}') {
                None
            } else {
                Some(self.parse_decimal_digits()?)
            }
        } else {
            Some(min)
        };
        self.expect('}')?;
        if max.is_some_and(|upper| min > upper) {
            return Err(RegExpError::new("invalid repetition count"));
        }
        Ok(RegExpQuantifier {
            min,
            max,
            greedy: true,
        })
    }

    fn parse_decimal_digits(&mut self) -> Result<u32, RegExpError> {
        let mut value = 0u32;
        let mut digits = 0usize;
        while let Some(ch) = self.peek() {
            let Some(digit) = ch.to_digit(10) else {
                break;
            };
            value = value
                .checked_mul(10)
                .and_then(|current| current.checked_add(digit))
                .ok_or_else(|| RegExpError::new("invalid repetition count"))?;
            digits += 1;
            self.bump();
        }
        if digits == 0 {
            return Err(RegExpError::new("invalid repetition count"));
        }
        Ok(value)
    }

    fn looks_like_quantifier_here(&self) -> bool {
        if self.peek() != Some('{') {
            return false;
        }
        let mut index = self.index + 1;
        let Some(first) = self.chars.get(index).copied() else {
            return false;
        };
        if !first.is_ascii_digit() {
            return false;
        }
        while self.chars.get(index).is_some_and(|ch| ch.is_ascii_digit()) {
            index += 1;
        }
        match self.chars.get(index).copied() {
            Some('}') => true,
            Some(',') => {
                index += 1;
                while self.chars.get(index).is_some_and(|ch| ch.is_ascii_digit()) {
                    index += 1;
                }
                self.chars.get(index).copied() == Some('}')
            }
            _ => false,
        }
    }

    fn scan_raw_escape(&mut self) -> Result<String, RegExpError> {
        let mut raw = String::new();
        raw.push(self.bump().unwrap());
        let Some(next) = self.bump() else {
            return Err(RegExpError::new("unexpected end of regular expression"));
        };
        raw.push(next);

        match next {
            'u' => {
                if self.peek() == Some('{') {
                    raw.push(self.bump().unwrap());
                    while let Some(ch) = self.peek() {
                        raw.push(ch);
                        self.bump();
                        if ch == '}' {
                            break;
                        }
                    }
                } else {
                    for _ in 0..4 {
                        let Some(ch) = self.peek() else {
                            break;
                        };
                        if !ch.is_ascii_hexdigit() {
                            break;
                        }
                        raw.push(ch);
                        self.bump();
                    }
                    if self.peek() == Some('\\')
                        && self.peek_n(1) == Some('u')
                        && (0..4).all(|offset| {
                            self.peek_n(offset + 2)
                                .is_some_and(|ch| ch.is_ascii_hexdigit())
                        })
                    {
                        raw.push(self.bump().unwrap());
                        raw.push(self.bump().unwrap());
                        for _ in 0..4 {
                            raw.push(self.bump().unwrap());
                        }
                    }
                }
            }
            'x' => {
                for _ in 0..2 {
                    let Some(ch) = self.peek() else {
                        break;
                    };
                    if !ch.is_ascii_hexdigit() {
                        break;
                    }
                    raw.push(ch);
                    self.bump();
                }
            }
            'p' | 'P' if self.peek() == Some('{') => {
                raw.push(self.bump().unwrap());
                while let Some(ch) = self.peek() {
                    raw.push(ch);
                    self.bump();
                    if ch == '}' {
                        break;
                    }
                }
            }
            '0'..='9' => {
                while let Some(ch) = self.peek() {
                    if !ch.is_ascii_digit() {
                        break;
                    }
                    raw.push(ch);
                    self.bump();
                }
            }
            'c' => {
                if let Some(ch) = self.peek() {
                    raw.push(ch);
                    self.bump();
                }
            }
            _ => {}
        }

        Ok(raw)
    }

    fn scan_character_class(&mut self) -> Result<String, RegExpError> {
        let mut raw = String::new();
        raw.push(self.bump().unwrap());
        if self.flags.unicode_sets {
            if self.peek() == Some('^') {
                raw.push(self.bump().unwrap());
            }
            self.scan_unicode_set_contents(&mut raw)?;
            return Ok(raw);
        }

        let mut escaped = false;
        loop {
            let Some(ch) = self.peek() else {
                return Err(RegExpError::new("unterminated character class"));
            };
            raw.push(ch);
            self.bump();

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                ']' => break,
                _ => {}
            }
        }
        Ok(raw)
    }

    fn scan_unicode_set_contents(&mut self, raw: &mut String) -> Result<(), RegExpError> {
        loop {
            let Some(ch) = self.peek() else {
                return Err(RegExpError::new("unterminated character class"));
            };
            match ch {
                '\\' => {
                    raw.push(self.bump().unwrap());
                    let Some(next) = self.peek() else {
                        return Err(RegExpError::new("unterminated character class"));
                    };
                    raw.push(next);
                    self.bump();
                    if next == 'q' && self.peek() == Some('{') {
                        raw.push(self.bump().unwrap());
                        self.scan_class_string_disjunction(raw)?;
                    }
                }
                '[' => {
                    raw.push(self.bump().unwrap());
                    if self.peek() == Some('^') {
                        raw.push(self.bump().unwrap());
                    }
                    self.scan_unicode_set_contents(raw)?;
                }
                ']' => {
                    raw.push(self.bump().unwrap());
                    return Ok(());
                }
                _ => {
                    raw.push(ch);
                    self.bump();
                }
            }
        }
    }

    fn scan_class_string_disjunction(&mut self, raw: &mut String) -> Result<(), RegExpError> {
        loop {
            let Some(ch) = self.peek() else {
                return Err(RegExpError::new("unterminated class string disjunction"));
            };
            raw.push(ch);
            self.bump();

            match ch {
                '\\' => {
                    let Some(next) = self.peek() else {
                        return Err(RegExpError::new("unterminated class string disjunction"));
                    };
                    raw.push(next);
                    self.bump();
                }
                '}' => return Ok(()),
                _ => {}
            }
        }
    }

    fn next_capture_index(&mut self) -> usize {
        self.capture_count += 1;
        self.capture_count
    }

    fn expect(&mut self, expected: char) -> Result<(), RegExpError> {
        match self.peek() {
            Some(ch) if ch == expected => {
                self.bump();
                Ok(())
            }
            _ => Err(RegExpError::new("syntax error")),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.chars.get(self.index + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
        Some(ch)
    }

    fn is_eof(&self) -> bool {
        self.index >= self.chars.len()
    }
}

fn validate_named_groups(pattern: &RegExpPattern) -> Result<(), RegExpError> {
    let mut occurrences = Vec::new();
    let mut backreferences = Vec::new();
    let mut next_disjunction_id = 0usize;
    collect_named_entities(
        &pattern.disjunction,
        &mut Vec::new(),
        &mut next_disjunction_id,
        &mut occurrences,
        &mut backreferences,
    );

    let mut by_name: HashMap<&str, Vec<Vec<(usize, usize)>>> = HashMap::new();
    for NamedGroupOccurrence { name, path } in occurrences {
        by_name.entry(name).or_default().push(path);
    }

    for entries in by_name.values() {
        for (index, left) in entries.iter().enumerate() {
            for right in &entries[index + 1..] {
                if can_both_participate(left, right) {
                    return Err(RegExpError::new("duplicate group name"));
                }
            }
        }
    }

    let has_named_groups = !by_name.is_empty();
    for PendingNamedBackreference {
        name,
        legacy_fallback,
    } in backreferences
    {
        if legacy_fallback && !has_named_groups {
            continue;
        }
        match name {
            Some(name) if by_name.contains_key(&name) => {}
            Some(_) => return Err(RegExpError::new("group name not defined")),
            None => return Err(RegExpError::new("invalid group name")),
        }
    }

    Ok(())
}

fn collect_named_entities<'a>(
    disjunction: &'a RegExpDisjunction,
    path: &mut Vec<(usize, usize)>,
    next_disjunction_id: &mut usize,
    occurrences: &mut Vec<NamedGroupOccurrence<'a>>,
    backreferences: &mut Vec<PendingNamedBackreference<'a>>,
) {
    let disjunction_id = if disjunction.alternatives.len() > 1 {
        let value = *next_disjunction_id;
        *next_disjunction_id += 1;
        Some(value)
    } else {
        None
    };

    for (alternative_index, alternative) in disjunction.alternatives.iter().enumerate() {
        let original_len = path.len();
        if let Some(id) = disjunction_id {
            path.push((id, alternative_index));
        }
        for term in &alternative.terms {
            match term {
                RegExpTerm::Assertion(RegExpAssertion::LookAhead { disjunction, .. })
                | RegExpTerm::Assertion(RegExpAssertion::LookBehind { disjunction, .. }) => {
                    collect_named_entities(
                        disjunction,
                        path,
                        next_disjunction_id,
                        occurrences,
                        backreferences,
                    );
                }
                RegExpTerm::Assertion(_) => {}
                RegExpTerm::Atom { atom, .. } => match atom {
                    RegExpAtom::Raw(_) => {}
                    RegExpAtom::NamedBackreference { name } => {
                        backreferences.push(PendingNamedBackreference {
                            name: Some(name),
                            legacy_fallback: false,
                        });
                    }
                    RegExpAtom::LegacyNamedEscape { name, .. } => {
                        backreferences.push(PendingNamedBackreference {
                            name: name.as_deref(),
                            legacy_fallback: true,
                        });
                    }
                    RegExpAtom::Group(group) => {
                        if let RegExpGroupKind::Capture { name: Some(name) } = &group.kind {
                            occurrences.push(NamedGroupOccurrence {
                                name,
                                path: path.clone(),
                            });
                        }
                        collect_named_entities(
                            &group.disjunction,
                            path,
                            next_disjunction_id,
                            occurrences,
                            backreferences,
                        );
                    }
                },
            }
        }
        path.truncate(original_len);
    }
}

fn can_both_participate(left: &[(usize, usize)], right: &[(usize, usize)]) -> bool {
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left.len() && right_index < right.len() {
        match left[left_index].0.cmp(&right[right_index].0) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                if left[left_index].1 != right[right_index].1 {
                    return false;
                }
                left_index += 1;
                right_index += 1;
            }
        }
    }

    true
}

fn validate_unicode_mode_raw_semantics(pattern: &RegExpPattern) -> Result<(), RegExpError> {
    if !pattern.flags.unicode_mode() {
        return Ok(());
    }

    validate_unicode_mode_disjunction(&pattern.disjunction, &pattern.flags, pattern.capture_count)
}

fn validate_unicode_mode_disjunction(
    disjunction: &RegExpDisjunction,
    flags: &RegExpFlags,
    capture_count: usize,
) -> Result<(), RegExpError> {
    for alternative in &disjunction.alternatives {
        for term in &alternative.terms {
            match term {
                RegExpTerm::Assertion(RegExpAssertion::LookAhead { disjunction, .. })
                | RegExpTerm::Assertion(RegExpAssertion::LookBehind { disjunction, .. }) => {
                    validate_unicode_mode_disjunction(disjunction, flags, capture_count)?;
                }
                RegExpTerm::Assertion(_) => {}
                RegExpTerm::Atom { atom, .. } => match atom {
                    RegExpAtom::Raw(raw) => {
                        validate_unicode_mode_raw_atom(raw, flags, capture_count)?
                    }
                    RegExpAtom::Group(group) => {
                        validate_unicode_mode_disjunction(
                            &group.disjunction,
                            flags,
                            capture_count,
                        )?;
                    }
                    RegExpAtom::NamedBackreference { .. }
                    | RegExpAtom::LegacyNamedEscape { .. } => {}
                },
            }
        }
    }

    Ok(())
}

fn validate_unicode_mode_raw_atom(
    raw: &str,
    flags: &RegExpFlags,
    capture_count: usize,
) -> Result<(), RegExpError> {
    if raw.starts_with('[') {
        if flags.unicode_sets {
            validate_unicode_sets_class_syntax(raw, flags, capture_count)
        } else {
            validate_unicode_class_syntax(raw, flags, capture_count)
        }
    } else if raw.starts_with('\\') {
        validate_escape_semantics(raw, flags, capture_count, false)
    } else {
        Ok(())
    }
}

fn validate_escape_semantics(
    raw: &str,
    flags: &RegExpFlags,
    capture_count: usize,
    in_class: bool,
) -> Result<(), RegExpError> {
    let mut chars = raw.chars();
    if chars.next() != Some('\\') {
        return Ok(());
    }
    let Some(kind) = chars.next() else {
        return Err(RegExpError::new("syntax error"));
    };

    match kind {
        'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'f' | 'n' | 'r' | 't' | 'v' => {
            if chars.next().is_none() {
                Ok(())
            } else {
                Err(RegExpError::new("syntax error"))
            }
        }
        'b' if in_class => {
            if chars.next().is_none() {
                Ok(())
            } else {
                Err(RegExpError::new("syntax error"))
            }
        }
        'c' => {
            if matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic())
                && chars.next().is_none()
            {
                Ok(())
            } else {
                Err(RegExpError::new("syntax error"))
            }
        }
        'x' => validate_hex_escape(raw),
        'u' => validate_unicode_escape(raw),
        'p' | 'P' => validate_property_escape_raw(raw, kind, flags),
        '0'..='9' => validate_decimal_escape(raw, capture_count),
        'k' => Err(RegExpError::new("invalid group name")),
        '-' if in_class && chars.next().is_none() => Ok(()),
        ch if is_valid_unicode_identity_escape(ch) && chars.next().is_none() => Ok(()),
        _ => Err(RegExpError::new("syntax error")),
    }
}

fn validate_hex_escape(raw: &str) -> Result<(), RegExpError> {
    let digits = raw
        .strip_prefix("\\x")
        .ok_or_else(|| RegExpError::new("syntax error"))?;
    if digits.len() == 2 && digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(RegExpError::new("syntax error"))
    }
}

fn validate_unicode_escape(raw: &str) -> Result<(), RegExpError> {
    let rest = raw
        .strip_prefix("\\u")
        .ok_or_else(|| RegExpError::new("syntax error"))?;
    if let Some(code_point) = rest.strip_prefix('{') {
        let digits = code_point
            .strip_suffix('}')
            .ok_or_else(|| RegExpError::new("syntax error"))?;
        if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(RegExpError::new("syntax error"));
        }
        let value =
            u32::from_str_radix(digits, 16).map_err(|_| RegExpError::new("syntax error"))?;
        if value > 0x10FFFF {
            return Err(RegExpError::new("syntax error"));
        }
        return Ok(());
    }

    let mut index = 0usize;
    while index < rest.len() {
        let digits = rest
            .get(index..index + 4)
            .ok_or_else(|| RegExpError::new("syntax error"))?;
        if !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(RegExpError::new("syntax error"));
        }
        index += 4;
        if index == rest.len() {
            return Ok(());
        }
        if !rest[index..].starts_with("\\u") {
            return Err(RegExpError::new("syntax error"));
        }
        index += 2;
    }

    Err(RegExpError::new("syntax error"))
}

fn validate_property_escape_raw(
    raw: &str,
    prefix: char,
    flags: &RegExpFlags,
) -> Result<(), RegExpError> {
    let Some(expression) = parse_property_escape(raw, prefix) else {
        return Err(RegExpError::new("syntax error"));
    };

    if is_property_of_strings_name(expression) {
        if flags.unicode_sets && prefix == 'p' {
            return Ok(());
        }
        return Err(RegExpError::new("invalid property of strings escape"));
    }

    if is_valid_character_property_expression(expression) {
        Ok(())
    } else {
        Err(RegExpError::new("syntax error"))
    }
}

fn validate_decimal_escape(raw: &str, capture_count: usize) -> Result<(), RegExpError> {
    let digits = raw
        .strip_prefix('\\')
        .ok_or_else(|| RegExpError::new("syntax error"))?;
    if digits == "0" {
        return Ok(());
    }
    if digits.starts_with('0') {
        return Err(RegExpError::new("syntax error"));
    }

    let value = digits
        .parse::<usize>()
        .map_err(|_| RegExpError::new("syntax error"))?;
    if value == 0 || value > capture_count {
        Err(RegExpError::new("syntax error"))
    } else {
        Ok(())
    }
}

fn validate_unicode_class_syntax(
    raw: &str,
    flags: &RegExpFlags,
    capture_count: usize,
) -> Result<(), RegExpError> {
    let mut atoms = Vec::new();
    let content_start = if raw.starts_with("[^") { 2 } else { 1 };
    let mut index = content_start;
    let content_end = raw.len() - 1;

    while index < content_end {
        if raw[index..].starts_with('\\') {
            let end = end_of_escape_fragment(raw, index)?;
            let escape = &raw[index..end];
            validate_escape_semantics(escape, flags, capture_count, true)?;
            atoms.push(ClassAtom {
                is_dash: false,
                singleton: !class_escape_is_multi_character(escape, flags),
            });
            index = end;
            continue;
        }

        let ch = raw[index..]
            .chars()
            .next()
            .ok_or_else(|| RegExpError::new("syntax error"))?;
        atoms.push(ClassAtom {
            is_dash: ch == '-',
            singleton: true,
        });
        index += ch.len_utf8();
    }

    for window in atoms.windows(3) {
        if window[1].is_dash && (!window[0].singleton || !window[2].singleton) {
            return Err(RegExpError::new("syntax error"));
        }
    }

    Ok(())
}

fn validate_unicode_sets_class_syntax(
    raw: &str,
    flags: &RegExpFlags,
    capture_count: usize,
) -> Result<(), RegExpError> {
    let content_start = if raw.starts_with("[^") { 2 } else { 1 };
    let contents = &raw[content_start..raw.len() - 1];
    let mut atoms = Vec::new();

    for operator in ["&&", "--"] {
        let parts = split_top_level_operator(contents, operator)?;
        if parts.len() > 1 && parts.iter().any(|part| part.is_empty()) {
            return Err(RegExpError::new("syntax error"));
        }
    }

    let mut index = 0usize;
    while index < contents.len() {
        if contents[index..].starts_with("\\q{") {
            atoms.push(ClassAtom {
                is_dash: false,
                singleton: false,
            });
            index = end_of_class_string_disjunction(contents, index + 3)?;
            continue;
        }

        if contents[index..].starts_with('\\') {
            let end = end_of_escape_fragment(contents, index)?;
            let escape = &contents[index..end];
            validate_escape_semantics(escape, flags, capture_count, true)?;
            atoms.push(ClassAtom {
                is_dash: false,
                singleton: !class_escape_is_multi_character(escape, flags),
            });
            index = end;
            continue;
        }

        if contents[index..].starts_with('[') {
            let end = end_of_unicode_sets_character_class(contents, index)?;
            let nested = &contents[index..end];
            if nested == "[]" || nested == "[^]" {
                return Err(RegExpError::new("syntax error"));
            }
            validate_unicode_sets_class_syntax(nested, flags, capture_count)?;
            atoms.push(ClassAtom {
                is_dash: false,
                singleton: false,
            });
            index = end;
            continue;
        }

        let ch = contents[index..]
            .chars()
            .next()
            .ok_or_else(|| RegExpError::new("syntax error"))?;
        let next = contents[index + ch.len_utf8()..].chars().next();

        if matches!(ch, '(' | ')' | '{' | '}' | '/' | '|') {
            return Err(RegExpError::new("syntax error"));
        }
        if ch == '-' {
            if next == Some('-') {
                index += 2;
                continue;
            }
            atoms.push(ClassAtom {
                is_dash: true,
                singleton: true,
            });
            index += 1;
            continue;
        }
        if ch == '&' && next == Some('&') {
            index += 2;
            continue;
        }
        if matches!(
            ch,
            '!' | '#'
                | '$'
                | '%'
                | '*'
                | '+'
                | ','
                | '.'
                | ':'
                | ';'
                | '<'
                | '='
                | '>'
                | '?'
                | '@'
                | '`'
                | '~'
                | '^'
        ) && next == Some(ch)
        {
            return Err(RegExpError::new("syntax error"));
        }

        atoms.push(ClassAtom {
            is_dash: false,
            singleton: true,
        });
        index += ch.len_utf8();
    }

    for (index, atom) in atoms.iter().enumerate() {
        if !atom.is_dash {
            continue;
        }
        let valid = index > 0
            && index + 1 < atoms.len()
            && !atoms[index - 1].is_dash
            && !atoms[index + 1].is_dash
            && atoms[index - 1].singleton
            && atoms[index + 1].singleton;
        if !valid {
            return Err(RegExpError::new("syntax error"));
        }
    }

    Ok(())
}

fn end_of_escape_fragment(source: &str, start: usize) -> Result<usize, RegExpError> {
    if !source[start..].starts_with('\\') {
        return Err(RegExpError::new("syntax error"));
    }

    let mut index = start + 1;
    let kind = source[index..]
        .chars()
        .next()
        .ok_or_else(|| RegExpError::new("syntax error"))?;
    index += kind.len_utf8();

    match kind {
        'u' => {
            if source[index..].starts_with('{') {
                index += 1;
                while index < source.len() {
                    let ch = source[index..]
                        .chars()
                        .next()
                        .ok_or_else(|| RegExpError::new("syntax error"))?;
                    index += ch.len_utf8();
                    if ch == '}' {
                        break;
                    }
                }
                Ok(index)
            } else {
                while index + 4 <= source.len()
                    && source[index..index + 4]
                        .chars()
                        .all(|ch| ch.is_ascii_hexdigit())
                {
                    index += 4;
                    if source[index..].starts_with("\\u")
                        && index + 6 <= source.len()
                        && source[index + 2..index + 6]
                            .chars()
                            .all(|ch| ch.is_ascii_hexdigit())
                    {
                        index += 2;
                    } else {
                        break;
                    }
                }
                Ok(index)
            }
        }
        'x' => {
            while index < source.len() {
                let ch = source[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| RegExpError::new("syntax error"))?;
                if !ch.is_ascii_hexdigit() {
                    break;
                }
                index += ch.len_utf8();
                if index - start >= 4 {
                    break;
                }
            }
            Ok(index)
        }
        'p' | 'P' if source[index..].starts_with('{') => {
            index += 1;
            while index < source.len() {
                let ch = source[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| RegExpError::new("syntax error"))?;
                index += ch.len_utf8();
                if ch == '}' {
                    break;
                }
            }
            Ok(index)
        }
        '0'..='9' => {
            while index < source.len() {
                let ch = source[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| RegExpError::new("syntax error"))?;
                if !ch.is_ascii_digit() {
                    break;
                }
                index += ch.len_utf8();
            }
            Ok(index)
        }
        'c' => {
            if index < source.len() {
                let ch = source[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| RegExpError::new("syntax error"))?;
                index += ch.len_utf8();
            }
            Ok(index)
        }
        _ => Ok(index),
    }
}

fn class_escape_is_multi_character(raw: &str, flags: &RegExpFlags) -> bool {
    match raw.as_bytes().get(1).copied() {
        Some(b'd' | b'D' | b's' | b'S' | b'w' | b'W') => true,
        Some(b'p' | b'P') if flags.unicode_mode() => true,
        _ => false,
    }
}

fn is_valid_unicode_identity_escape(ch: char) -> bool {
    matches!(
        ch,
        '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/'
    )
}

#[derive(Clone, Copy)]
struct ClassAtom {
    is_dash: bool,
    singleton: bool,
}

fn validate_unicode_sets_early_errors(pattern: &RegExpPattern) -> Result<(), RegExpError> {
    if !pattern.flags.unicode_sets {
        return Ok(());
    }

    validate_unicode_sets_disjunction(&pattern.disjunction)
}

fn validate_unicode_sets_disjunction(disjunction: &RegExpDisjunction) -> Result<(), RegExpError> {
    for alternative in &disjunction.alternatives {
        for term in &alternative.terms {
            match term {
                RegExpTerm::Assertion(RegExpAssertion::LookAhead { disjunction, .. })
                | RegExpTerm::Assertion(RegExpAssertion::LookBehind { disjunction, .. }) => {
                    validate_unicode_sets_disjunction(disjunction)?;
                }
                RegExpTerm::Assertion(_) => {}
                RegExpTerm::Atom { atom, .. } => match atom {
                    RegExpAtom::Raw(raw) => validate_unicode_sets_raw_atom(raw)?,
                    RegExpAtom::Group(group) => {
                        validate_unicode_sets_disjunction(&group.disjunction)?;
                    }
                    RegExpAtom::NamedBackreference { .. }
                    | RegExpAtom::LegacyNamedEscape { .. } => {}
                },
            }
        }
    }

    Ok(())
}

fn validate_unicode_sets_raw_atom(raw: &str) -> Result<(), RegExpError> {
    if let Some(property) = parse_property_escape(raw, 'P')
        && is_property_of_strings_name(property)
    {
        return Err(RegExpError::new("invalid property of strings escape"));
    }

    if raw.starts_with('[') {
        validate_unicode_sets_character_class(raw)?;
    }

    Ok(())
}

fn validate_unicode_sets_character_class(raw: &str) -> Result<bool, RegExpError> {
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return Ok(false);
    }

    let is_negated = raw.starts_with("[^");
    let content_start = if is_negated { 2 } else { 1 };
    let contents = &raw[content_start..raw.len() - 1];
    let may_contain_strings = unicode_sets_contents_may_contain_strings(contents)?;

    if is_negated && may_contain_strings {
        return Err(RegExpError::new(
            "negated character class cannot contain strings",
        ));
    }

    if is_negated {
        Ok(false)
    } else {
        Ok(may_contain_strings)
    }
}

fn unicode_sets_contents_may_contain_strings(contents: &str) -> Result<bool, RegExpError> {
    let subtraction_parts = split_top_level_operator(contents, "--")?;
    if subtraction_parts.len() > 1 {
        let left_may_contain_strings =
            unicode_sets_contents_may_contain_strings(subtraction_parts[0])?;
        for part in subtraction_parts.into_iter().skip(1) {
            unicode_sets_contents_may_contain_strings(part)?;
        }
        return Ok(left_may_contain_strings);
    }

    let intersection_parts = split_top_level_operator(contents, "&&")?;
    if intersection_parts.len() > 1 {
        let mut all_may_contain_strings = true;
        for part in intersection_parts {
            all_may_contain_strings &= unicode_sets_contents_may_contain_strings(part)?;
        }
        return Ok(all_may_contain_strings);
    }

    unicode_sets_union_may_contain_strings(contents)
}

fn unicode_sets_union_may_contain_strings(contents: &str) -> Result<bool, RegExpError> {
    let mut index = 0usize;
    while index < contents.len() {
        if contents[index..].starts_with("\\q{") {
            let end = end_of_class_string_disjunction(contents, index + 3)?;
            let q_contents = &contents[index + 3..end - 1];
            if class_string_disjunction_may_contain_strings(q_contents)? {
                return Ok(true);
            }
            index = end;
            continue;
        }

        if let Some(property) = parse_property_escape_at(contents, index, 'P') {
            if is_property_of_strings_name(property.expression) {
                return Err(RegExpError::new("invalid property of strings escape"));
            }
            index = property.end;
            continue;
        }

        if let Some(property) = parse_property_escape_at(contents, index, 'p') {
            if is_property_of_strings_name(property.expression) {
                return Ok(true);
            }
            index = property.end;
            continue;
        }

        if contents[index..].starts_with('[') {
            let end = end_of_unicode_sets_character_class(contents, index)?;
            let nested = &contents[index..end];
            if validate_unicode_sets_character_class(nested)? {
                return Ok(true);
            }
            index = end;
            continue;
        }

        if contents[index..].starts_with('\\') {
            index = skip_general_escape(contents, index)?;
            continue;
        }

        index += next_char_len(contents, index)?;
    }

    Ok(false)
}

fn split_top_level_operator<'a>(
    contents: &'a str,
    operator: &str,
) -> Result<Vec<&'a str>, RegExpError> {
    let mut parts = Vec::new();
    let mut last = 0usize;
    let mut index = 0usize;
    let mut nested_class_depth = 0usize;

    while index < contents.len() {
        if contents[index..].starts_with("\\q{") {
            index = end_of_class_string_disjunction(contents, index + 3)?;
            continue;
        }

        if contents[index..].starts_with('\\') {
            index = skip_general_escape(contents, index)?;
            continue;
        }

        let ch = contents[index..]
            .chars()
            .next()
            .ok_or_else(|| RegExpError::new("syntax error"))?;

        match ch {
            '[' => {
                nested_class_depth += 1;
                index += ch.len_utf8();
            }
            ']' => {
                nested_class_depth = nested_class_depth.saturating_sub(1);
                index += ch.len_utf8();
            }
            _ if nested_class_depth == 0 && contents[index..].starts_with(operator) => {
                parts.push(&contents[last..index]);
                index += operator.len();
                last = index;
            }
            _ => {
                index += ch.len_utf8();
            }
        }
    }

    if !parts.is_empty() {
        parts.push(&contents[last..]);
        return Ok(parts);
    }

    Ok(vec![contents])
}

fn class_string_disjunction_may_contain_strings(contents: &str) -> Result<bool, RegExpError> {
    let mut alternative_length = 0usize;
    let mut index = 0usize;

    while index < contents.len() {
        if contents[index..].starts_with('|') {
            if alternative_length != 1 {
                return Ok(true);
            }
            alternative_length = 0;
            index += 1;
            continue;
        }

        if contents[index..].starts_with("\\u{") {
            alternative_length += 1;
            index += 3;
            while index < contents.len() {
                let ch = contents[index..]
                    .chars()
                    .next()
                    .ok_or_else(|| RegExpError::new("syntax error"))?;
                index += ch.len_utf8();
                if ch == '}' {
                    break;
                }
            }
            continue;
        }

        if contents[index..].starts_with('\\') {
            alternative_length += 1;
            index = skip_general_escape(contents, index)?;
            continue;
        }

        alternative_length += 1;
        index += next_char_len(contents, index)?;
    }

    Ok(alternative_length != 1)
}

fn parse_property_escape(raw: &str, prefix: char) -> Option<&str> {
    parse_property_escape_at(raw, 0, prefix)
        .filter(|escape| escape.end == raw.len())
        .map(|escape| escape.expression)
}

struct PropertyEscape<'a> {
    expression: &'a str,
    end: usize,
}

fn parse_property_escape_at<'a>(
    source: &'a str,
    start: usize,
    prefix: char,
) -> Option<PropertyEscape<'a>> {
    let header = match prefix {
        'p' => "\\p{",
        'P' => "\\P{",
        _ => return None,
    };
    if !source[start..].starts_with(header) {
        return None;
    }

    let expression_start = start + header.len();
    let rest = &source[expression_start..];
    let end_offset = rest.find('}')?;
    let end = expression_start + end_offset + 1;
    Some(PropertyEscape {
        expression: &source[expression_start..expression_start + end_offset],
        end,
    })
}

fn is_property_of_strings_name(expression: &str) -> bool {
    property_of_strings_expressions().contains(expression)
}

fn is_valid_character_property_expression(expression: &str) -> bool {
    valid_character_property_expressions().contains(expression)
}

fn property_of_strings_expressions() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| PROPERTY_OF_STRINGS_EXPRESSIONS.iter().copied().collect())
}

fn valid_character_property_expressions() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        VALID_CHARACTER_PROPERTY_EXPRESSIONS
            .iter()
            .copied()
            .collect()
    })
}

fn end_of_unicode_sets_character_class(source: &str, start: usize) -> Result<usize, RegExpError> {
    let mut index = start + 1;
    if source[index..].starts_with('^') {
        index += 1;
    }

    while index < source.len() {
        if source[index..].starts_with("\\q{") {
            index = end_of_class_string_disjunction(source, index + 3)?;
            continue;
        }

        if source[index..].starts_with('\\') {
            index = skip_general_escape(source, index)?;
            continue;
        }

        let ch = source[index..]
            .chars()
            .next()
            .ok_or_else(|| RegExpError::new("syntax error"))?;

        match ch {
            '[' => index = end_of_unicode_sets_character_class(source, index)?,
            ']' => return Ok(index + 1),
            _ => index += ch.len_utf8(),
        }
    }

    Err(RegExpError::new("unterminated character class"))
}

fn end_of_class_string_disjunction(source: &str, start: usize) -> Result<usize, RegExpError> {
    let mut index = start;
    while index < source.len() {
        let ch = source[index..]
            .chars()
            .next()
            .ok_or_else(|| RegExpError::new("syntax error"))?;
        index += ch.len_utf8();

        if ch == '\\' {
            index += next_char_len(source, index)?;
            continue;
        }

        if ch == '}' {
            return Ok(index);
        }
    }

    Err(RegExpError::new("unterminated class string disjunction"))
}

fn skip_general_escape(source: &str, start: usize) -> Result<usize, RegExpError> {
    let mut index = start + 1;
    index += next_char_len(source, index)?;
    Ok(index)
}

fn next_char_len(source: &str, index: usize) -> Result<usize, RegExpError> {
    source[index..]
        .chars()
        .next()
        .map(char::len_utf8)
        .ok_or_else(|| RegExpError::new("syntax error"))
}

fn emit_disjunction(disjunction: &RegExpDisjunction) -> String {
    disjunction
        .alternatives
        .iter()
        .map(emit_alternative)
        .collect::<Vec<_>>()
        .join("|")
}

fn emit_alternative(alternative: &RegExpAlternative) -> String {
    let mut out = String::new();
    for term in &alternative.terms {
        match term {
            RegExpTerm::Assertion(assertion) => {
                out.push_str(&emit_assertion(assertion));
            }
            RegExpTerm::Atom { atom, quantifier } => {
                out.push_str(&emit_atom(atom));
                if let Some(quantifier) = quantifier {
                    out.push_str(&emit_quantifier(quantifier));
                }
            }
        }
    }
    out
}

fn emit_assertion(assertion: &RegExpAssertion) -> String {
    match assertion {
        RegExpAssertion::Start => "^".to_string(),
        RegExpAssertion::End => "$".to_string(),
        RegExpAssertion::WordBoundary => "\\b".to_string(),
        RegExpAssertion::NonWordBoundary => "\\B".to_string(),
        RegExpAssertion::LookAhead {
            negative,
            disjunction,
        } => {
            let prefix = if *negative { "(?!" } else { "(?=" };
            format!("{prefix}{})", emit_disjunction(disjunction))
        }
        RegExpAssertion::LookBehind {
            negative,
            disjunction,
        } => {
            let prefix = if *negative { "(?<!" } else { "(?<=" };
            format!("{prefix}{})", emit_disjunction(disjunction))
        }
    }
}

fn emit_assertion_as_group(assertion: &RegExpAssertion) -> String {
    match assertion {
        RegExpAssertion::LookAhead {
            negative,
            disjunction,
        } => {
            let prefix = if *negative { "(?!" } else { "(?=" };
            format!("{prefix}{})", emit_disjunction(disjunction))
        }
        _ => String::new(),
    }
}

fn emit_atom(atom: &RegExpAtom) -> String {
    match atom {
        RegExpAtom::Raw(raw) => raw.clone(),
        RegExpAtom::NamedBackreference { name } => format!("\\k<{name}>"),
        RegExpAtom::LegacyNamedEscape { raw, .. } => raw.clone(),
        RegExpAtom::Group(group) => {
            let prefix = match &group.kind {
                RegExpGroupKind::Capture { name } => match name {
                    Some(name) => {
                        return format!("(?<{name}>{})", emit_disjunction(&group.disjunction));
                    }
                    None => "(",
                },
                RegExpGroupKind::NonCapture => "(?:",
                RegExpGroupKind::Modifiers { .. } => "(?:",
            };
            format!("{prefix}{})", emit_disjunction(&group.disjunction))
        }
    }
}

fn emit_quantifier(quantifier: &RegExpQuantifier) -> String {
    let mut out = match (quantifier.min, quantifier.max) {
        (0, None) => "*".to_string(),
        (1, None) => "+".to_string(),
        (0, Some(1)) => "?".to_string(),
        (min, Some(max)) if min == max => format!("{{{min}}}"),
        (min, None) => format!("{{{min},}}"),
        (min, Some(max)) => format!("{{{min},{max}}}"),
    };
    if !quantifier.greedy {
        out.push('?');
    }
    out
}

fn has_duplicate_modifier(value: &str) -> bool {
    let mut seen = [false; 3];
    for ch in value.chars() {
        let index = match ch {
            'i' => 0,
            'm' => 1,
            's' => 2,
            _ => return true,
        };
        if seen[index] {
            return true;
        }
        seen[index] = true;
    }
    false
}

fn is_regexp_syntax_character(ch: char) -> bool {
    matches!(
        ch,
        '^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
    )
}

fn is_legacy_named_escape_terminator(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')' | '[' | ']' | '{' | '}' | '|' | '*' | '+' | '?' | '^' | '$'
    )
}

fn is_identifier_start(ch: char) -> bool {
    ch == '$' || ch == '_' || is_xid_start(ch) || is_other_id_start(ch)
}

fn is_identifier_part(ch: char) -> bool {
    ch == '$'
        || ch == '_'
        || is_xid_continue(ch)
        || is_other_id_continue(ch)
        || matches!(ch, '\u{200c}' | '\u{200d}')
}

fn is_other_id_start(ch: char) -> bool {
    matches!(
        ch,
        '\u{1885}' | '\u{1886}' | '\u{2118}' | '\u{212e}' | '\u{309b}' | '\u{309c}'
    )
}

fn is_other_id_continue(ch: char) -> bool {
    is_other_id_start(ch)
        || matches!(
            ch,
            '\u{00b7}' | '\u{0387}' | '\u{1369}'..='\u{1371}' | '\u{19da}'
        )
}
