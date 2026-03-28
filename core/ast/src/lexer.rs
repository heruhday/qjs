use std::{error::Error, fmt};

use unicode_ident::{is_xid_continue, is_xid_start};

use crate::token::{Position, Span, Token, TokenKind, TokenTag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

impl LexError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {} column {}",
            self.message, self.span.start.line, self.span.start.column
        )
    }
}

impl Error for LexError {}

pub struct Lexer<'a> {
    source: &'a str,
    offset: usize,
    line: usize,
    column: usize,
    last_token: Option<TokenTag>,
    allow_html_comments: bool,
    html_close_comment_allowed: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self::new_with_html_comments(source, true)
    }

    pub fn new_with_html_comments(source: &'a str, allow_html_comments: bool) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
            last_token: None,
            allow_html_comments,
            html_close_comment_allowed: true,
        }
    }

    pub fn scan_all(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            let token = self.next_token()?;
            let is_eof = token.tag == TokenTag::Eof;
            self.last_token = (!is_eof).then_some(token.tag);
            tokens.push(token);
            if is_eof {
                break;
            }
        }

        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        let leading_line_break = self.skip_whitespace_and_comments()?;
        let start = self.position();

        let token = match self.peek_char() {
            None => self.make_token(TokenKind::Eof, start, leading_line_break),
            Some('(') => self.simple_token(TokenKind::LeftParen, start, leading_line_break),
            Some(')') => self.simple_token(TokenKind::RightParen, start, leading_line_break),
            Some('{') => self.simple_token(TokenKind::LeftBrace, start, leading_line_break),
            Some('}') => self.simple_token(TokenKind::RightBrace, start, leading_line_break),
            Some('[') => self.simple_token(TokenKind::LeftBracket, start, leading_line_break),
            Some(']') => self.simple_token(TokenKind::RightBracket, start, leading_line_break),
            Some(';') => self.simple_token(TokenKind::Semicolon, start, leading_line_break),
            Some(',') => self.simple_token(TokenKind::Comma, start, leading_line_break),
            Some(':') => self.simple_token(TokenKind::Colon, start, leading_line_break),
            Some('~') => self.simple_token(TokenKind::BitNot, start, leading_line_break),
            Some('@') => self.simple_token(TokenKind::At, start, leading_line_break),
            Some('.') if self.starts_with("...") => {
                self.consume_str("...");
                self.make_token(TokenKind::Ellipsis, start, leading_line_break)
            }
            Some('.') if self.peek_nth_char(1).is_some_and(|ch| ch.is_ascii_digit()) => {
                return self.lex_number(start, leading_line_break, true);
            }
            Some('.') => self.simple_token(TokenKind::Dot, start, leading_line_break),
            Some('+') if self.starts_with("++") => {
                self.consume_str("++");
                self.make_token(TokenKind::Increment, start, leading_line_break)
            }
            Some('+') if self.starts_with("+=") => {
                self.consume_str("+=");
                self.make_token(TokenKind::AddAssign, start, leading_line_break)
            }
            Some('+') => self.simple_token(TokenKind::Add, start, leading_line_break),
            Some('-') if self.starts_with("--") => {
                self.consume_str("--");
                self.make_token(TokenKind::Decrement, start, leading_line_break)
            }
            Some('-') if self.starts_with("-=") => {
                self.consume_str("-=");
                self.make_token(TokenKind::SubAssign, start, leading_line_break)
            }
            Some('-') => self.simple_token(TokenKind::Sub, start, leading_line_break),
            Some('*') if self.starts_with("**=") => {
                self.consume_str("**=");
                self.make_token(TokenKind::PowAssign, start, leading_line_break)
            }
            Some('*') if self.starts_with("**") => {
                self.consume_str("**");
                self.make_token(TokenKind::Pow, start, leading_line_break)
            }
            Some('*') if self.starts_with("*=") => {
                self.consume_str("*=");
                self.make_token(TokenKind::MulAssign, start, leading_line_break)
            }
            Some('*') => self.simple_token(TokenKind::Mul, start, leading_line_break),
            Some('/') if self.regex_allowed() && self.prefer_division_over_regex() => {
                self.simple_token(TokenKind::Div, start, leading_line_break)
            }
            Some('/') if self.regex_allowed() => {
                let checkpoint = (self.offset, self.line, self.column);
                match self.lex_regexp(start, leading_line_break) {
                    Ok(token) => token,
                    Err(error) if error.message == "unterminated regexp literal" => {
                        (self.offset, self.line, self.column) = checkpoint;
                        self.simple_token(TokenKind::Div, start, leading_line_break)
                    }
                    Err(error) => return Err(error),
                }
            }
            Some('/') if self.starts_with("/=") => {
                self.consume_str("/=");
                self.make_token(TokenKind::DivAssign, start, leading_line_break)
            }
            Some('/') => self.simple_token(TokenKind::Div, start, leading_line_break),
            Some('%') if self.starts_with("%=") => {
                self.consume_str("%=");
                self.make_token(TokenKind::ModAssign, start, leading_line_break)
            }
            Some('%') => self.simple_token(TokenKind::Mod, start, leading_line_break),
            Some('<') if self.starts_with("<<=") => {
                self.consume_str("<<=");
                self.make_token(TokenKind::ShlAssign, start, leading_line_break)
            }
            Some('<') if self.starts_with("<<") => {
                self.consume_str("<<");
                self.make_token(TokenKind::Shl, start, leading_line_break)
            }
            Some('<') if self.starts_with("<=") => {
                self.consume_str("<=");
                self.make_token(TokenKind::Lte, start, leading_line_break)
            }
            Some('<') => self.simple_token(TokenKind::Lt, start, leading_line_break),
            Some('>') if self.starts_with(">>>=") => {
                self.consume_str(">>>=");
                self.make_token(TokenKind::ShrAssign, start, leading_line_break)
            }
            Some('>') if self.starts_with(">>>") => {
                self.consume_str(">>>");
                self.make_token(TokenKind::Shr, start, leading_line_break)
            }
            Some('>') if self.starts_with(">>=") => {
                self.consume_str(">>=");
                self.make_token(TokenKind::SarAssign, start, leading_line_break)
            }
            Some('>') if self.starts_with(">>") => {
                self.consume_str(">>");
                self.make_token(TokenKind::Sar, start, leading_line_break)
            }
            Some('>') if self.starts_with(">=") => {
                self.consume_str(">=");
                self.make_token(TokenKind::Gte, start, leading_line_break)
            }
            Some('>') => self.simple_token(TokenKind::Gt, start, leading_line_break),
            Some('&') if self.starts_with("&&=") => {
                self.consume_str("&&=");
                self.make_token(TokenKind::LogicalAndAssign, start, leading_line_break)
            }
            Some('&') if self.starts_with("&&") => {
                self.consume_str("&&");
                self.make_token(TokenKind::LogicalAnd, start, leading_line_break)
            }
            Some('&') if self.starts_with("&=") => {
                self.consume_str("&=");
                self.make_token(TokenKind::AndAssign, start, leading_line_break)
            }
            Some('&') => self.simple_token(TokenKind::BitAnd, start, leading_line_break),
            Some('|') if self.starts_with("||=") => {
                self.consume_str("||=");
                self.make_token(TokenKind::LogicalOrAssign, start, leading_line_break)
            }
            Some('|') if self.starts_with("||") => {
                self.consume_str("||");
                self.make_token(TokenKind::LogicalOr, start, leading_line_break)
            }
            Some('|') if self.starts_with("|=") => {
                self.consume_str("|=");
                self.make_token(TokenKind::OrAssign, start, leading_line_break)
            }
            Some('|') => self.simple_token(TokenKind::BitOr, start, leading_line_break),
            Some('^') if self.starts_with("^=") => {
                self.consume_str("^=");
                self.make_token(TokenKind::XorAssign, start, leading_line_break)
            }
            Some('^') => self.simple_token(TokenKind::BitXor, start, leading_line_break),
            Some('?') if self.starts_with("??=") => {
                self.consume_str("??=");
                self.make_token(TokenKind::NullishAssign, start, leading_line_break)
            }
            Some('?') if self.starts_with("??") => {
                self.consume_str("??");
                self.make_token(TokenKind::NullishCoalescing, start, leading_line_break)
            }
            Some('?')
                if self.starts_with("?.")
                    && !self.peek_nth_char(2).is_some_and(|ch| ch.is_ascii_digit()) =>
            {
                self.consume_str("?.");
                self.make_token(TokenKind::OptionalChain, start, leading_line_break)
            }
            Some('?') => self.simple_token(TokenKind::Question, start, leading_line_break),
            Some('=') if self.starts_with("===") => {
                self.consume_str("===");
                self.make_token(TokenKind::StrictEq, start, leading_line_break)
            }
            Some('=') if self.starts_with("==") => {
                self.consume_str("==");
                self.make_token(TokenKind::Eq, start, leading_line_break)
            }
            Some('=') if self.starts_with("=>") => {
                self.consume_str("=>");
                self.make_token(TokenKind::Arrow, start, leading_line_break)
            }
            Some('=') => self.simple_token(TokenKind::Assign, start, leading_line_break),
            Some('!') if self.starts_with("!==") => {
                self.consume_str("!==");
                self.make_token(TokenKind::StrictNe, start, leading_line_break)
            }
            Some('!') if self.starts_with("!=") => {
                self.consume_str("!=");
                self.make_token(TokenKind::Ne, start, leading_line_break)
            }
            Some('!') => self.simple_token(TokenKind::Not, start, leading_line_break),
            Some('"') | Some('\'') => self.lex_string(start, leading_line_break)?,
            Some('`') => self.lex_template(start, leading_line_break)?,
            Some('#') => self.lex_private_name(start, leading_line_break)?,
            Some(ch) if ch.is_ascii_digit() => self.lex_number(start, leading_line_break, false)?,
            Some('\\') if self.looks_like_identifier_escape() => {
                self.lex_identifier(start, leading_line_break)?
            }
            Some(ch) if is_identifier_start(ch) => {
                self.lex_identifier(start, leading_line_break)?
            }
            Some(ch) => {
                return Err(
                    self.error_at(format!("unexpected character '{ch}'"), Span::point(start))
                );
            }
        };

        if !matches!(token.kind, TokenKind::Eof) {
            self.html_close_comment_allowed = false;
        }

        Ok(token)
    }

    fn lex_number(
        &mut self,
        start: Position,
        leading_line_break: bool,
        started_with_dot: bool,
    ) -> Result<Token, LexError> {
        let mut raw = String::new();

        if started_with_dot {
            raw.push('.');
            self.bump_char();
            self.consume_decimal_digits(&mut raw);
            if matches!(self.peek_char(), Some('e' | 'E')) {
                raw.push(self.bump_char().expect("peeked exponent marker"));
                if matches!(self.peek_char(), Some('+' | '-')) {
                    raw.push(self.bump_char().expect("peeked exponent sign"));
                }
                let before = raw.len();
                self.consume_decimal_digits(&mut raw);
                if raw.len() == before {
                    return Err(self.error_at(
                        "expected digits in numeric exponent",
                        Span::point(self.position()),
                    ));
                }
            }
        } else if self.starts_with("0x") || self.starts_with("0X") {
            raw.push_str(&self.source[self.offset..self.offset + 2]);
            self.consume_str(&self.source[self.offset..self.offset + 2]);
            let digits = self.consume_radix_digits(&mut raw, 16);
            if digits == 0 {
                return Err(self.error_at("expected hexadecimal digits", Span::point(start)));
            }
        } else if self.starts_with("0o") || self.starts_with("0O") {
            raw.push_str(&self.source[self.offset..self.offset + 2]);
            self.consume_str(&self.source[self.offset..self.offset + 2]);
            let digits = self.consume_radix_digits(&mut raw, 8);
            if digits == 0 {
                return Err(self.error_at("expected octal digits", Span::point(start)));
            }
        } else if self.starts_with("0b") || self.starts_with("0B") {
            raw.push_str(&self.source[self.offset..self.offset + 2]);
            self.consume_str(&self.source[self.offset..self.offset + 2]);
            let digits = self.consume_radix_digits(&mut raw, 2);
            if digits == 0 {
                return Err(self.error_at("expected binary digits", Span::point(start)));
            }
        } else {
            self.consume_decimal_digits(&mut raw);
            if self.peek_char() == Some('.')
                && self.peek_nth_char(1) != Some('?')
                && !(self.peek_nth_char(1) == Some('.') && self.peek_nth_char(2) == Some('.'))
            {
                raw.push('.');
                self.bump_char();
                self.consume_decimal_digits(&mut raw);
            }
            if matches!(self.peek_char(), Some('e' | 'E')) {
                raw.push(self.bump_char().expect("peeked exponent marker"));
                if matches!(self.peek_char(), Some('+' | '-')) {
                    raw.push(self.bump_char().expect("peeked exponent sign"));
                }
                let before = raw.len();
                self.consume_decimal_digits(&mut raw);
                if raw.len() == before {
                    return Err(self.error_at(
                        "expected digits in numeric exponent",
                        Span::point(self.position()),
                    ));
                }
            }
        }

        if self.peek_char() == Some('n') {
            raw.push('n');
            self.bump_char();
        }

        self.validate_numeric_literal(&raw, start)?;
        Ok(self.make_token(TokenKind::Number(raw), start, leading_line_break))
    }

    fn lex_string(&mut self, start: Position, leading_line_break: bool) -> Result<Token, LexError> {
        let quote = self.bump_char().expect("string quote already checked");
        let mut value = String::new();

        loop {
            match self.peek_char() {
                None => {
                    return Err(self.error_at("unterminated string literal", Span::point(start)));
                }
                Some(ch) if ch == quote => {
                    self.bump_char();
                    break;
                }
                Some('\n' | '\r') => {
                    return Err(
                        self.error_at("unterminated string literal", Span::point(self.position()))
                    );
                }
                Some('\\') => {
                    self.bump_char();
                    if let Some(escaped) = self.read_escape_sequence()? {
                        value.push_str(&escaped);
                    }
                }
                Some(ch) => {
                    value.push(ch);
                    self.bump_char();
                }
            }
        }

        Ok(self.make_token(TokenKind::String(value), start, leading_line_break))
    }

    fn lex_template(
        &mut self,
        start: Position,
        leading_line_break: bool,
    ) -> Result<Token, LexError> {
        self.bump_char();
        let mut value = String::new();
        let mut invalid_escape = false;

        loop {
            match self.peek_char() {
                None => {
                    return Err(self.error_at("unterminated template literal", Span::point(start)));
                }
                Some('`') => {
                    self.bump_char();
                    break;
                }
                Some('$') if self.peek_nth_char(1) == Some('{') => {
                    value.push('$');
                    value.push('{');
                    self.consume_str("${");
                    self.copy_template_expression(&mut value, 1)?;
                }
                Some('\\') => {
                    self.bump_char();
                    let (escaped, escape_invalid) = self.read_template_escape_sequence()?;
                    invalid_escape |= escape_invalid;
                    if let Some(escaped) = escaped {
                        value.push_str(&escaped);
                    }
                }
                Some(ch) if is_line_terminator(ch) => {
                    value.push('\n');
                    self.consume_line_break();
                }
                Some(ch) => {
                    value.push(ch);
                    self.bump_char();
                }
            }
        }

        Ok(self.make_token(
            TokenKind::Template {
                value,
                invalid_escape,
            },
            start,
            leading_line_break,
        ))
    }

    fn copy_template_expression(
        &mut self,
        buffer: &mut String,
        mut depth: usize,
    ) -> Result<(), LexError> {
        while depth > 0 {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_at(
                    "unterminated template interpolation",
                    Span::point(self.position()),
                ));
            };

            match ch {
                '\'' | '"' => {
                    buffer.push(ch);
                    self.bump_char();
                    self.copy_string_contents(buffer, ch)?;
                }
                '`' => {
                    buffer.push('`');
                    self.bump_char();
                    self.copy_nested_template(buffer)?;
                }
                '/' if self.starts_with("//") => {
                    buffer.push('/');
                    buffer.push('/');
                    self.consume_str("//");
                    self.copy_line_comment(buffer);
                }
                '/' if self.starts_with("/*") => {
                    buffer.push('/');
                    buffer.push('*');
                    self.consume_str("/*");
                    self.copy_block_comment(buffer)?;
                }
                '{' => {
                    depth += 1;
                    buffer.push('{');
                    self.bump_char();
                }
                '}' => {
                    depth -= 1;
                    buffer.push('}');
                    self.bump_char();
                }
                c if is_line_terminator(c) => {
                    buffer.push('\n');
                    self.consume_line_break();
                }
                _ => {
                    buffer.push(ch);
                    self.bump_char();
                }
            }
        }

        Ok(())
    }

    fn copy_nested_template(&mut self, buffer: &mut String) -> Result<(), LexError> {
        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_at(
                    "unterminated template literal",
                    Span::point(self.position()),
                ));
            };

            match ch {
                '`' => {
                    buffer.push('`');
                    self.bump_char();
                    break;
                }
                '$' if self.peek_nth_char(1) == Some('{') => {
                    buffer.push('$');
                    buffer.push('{');
                    self.consume_str("${");
                    self.copy_template_expression(buffer, 1)?;
                }
                '\\' => {
                    buffer.push('\\');
                    self.bump_char();
                    let Some(next) = self.peek_char() else {
                        return Err(self.error_at(
                            "unterminated escape sequence",
                            Span::point(self.position()),
                        ));
                    };
                    if is_line_terminator(next) {
                        buffer.push('\n');
                        self.consume_line_break();
                    } else {
                        buffer.push(next);
                        self.bump_char();
                    }
                }
                c if is_line_terminator(c) => {
                    buffer.push('\n');
                    self.consume_line_break();
                }
                _ => {
                    buffer.push(ch);
                    self.bump_char();
                }
            }
        }

        Ok(())
    }

    fn copy_string_contents(&mut self, buffer: &mut String, quote: char) -> Result<(), LexError> {
        loop {
            let Some(ch) = self.peek_char() else {
                return Err(
                    self.error_at("unterminated string literal", Span::point(self.position()))
                );
            };

            match ch {
                c if c == quote => {
                    buffer.push(c);
                    self.bump_char();
                    break;
                }
                '\\' => {
                    buffer.push('\\');
                    self.bump_char();
                    let Some(next) = self.peek_char() else {
                        return Err(self.error_at(
                            "unterminated escape sequence",
                            Span::point(self.position()),
                        ));
                    };
                    if is_line_terminator(next) {
                        buffer.push('\n');
                        self.consume_line_break();
                    } else {
                        buffer.push(next);
                        self.bump_char();
                    }
                }
                '\n' | '\r' => {
                    return Err(
                        self.error_at("unterminated string literal", Span::point(self.position()))
                    );
                }
                _ => {
                    buffer.push(ch);
                    self.bump_char();
                }
            }
        }

        Ok(())
    }

    fn copy_line_comment(&mut self, buffer: &mut String) {
        while let Some(ch) = self.peek_char() {
            if is_line_terminator(ch) {
                break;
            }
            buffer.push(ch);
            self.bump_char();
        }
    }

    fn copy_block_comment(&mut self, buffer: &mut String) -> Result<(), LexError> {
        loop {
            let Some(ch) = self.peek_char() else {
                return Err(
                    self.error_at("unterminated block comment", Span::point(self.position()))
                );
            };

            if self.starts_with("*/") {
                buffer.push('*');
                buffer.push('/');
                self.consume_str("*/");
                break;
            }

            if is_line_terminator(ch) {
                buffer.push('\n');
                self.consume_line_break();
            } else {
                buffer.push(ch);
                self.bump_char();
            }
        }

        Ok(())
    }

    fn lex_regexp(&mut self, start: Position, leading_line_break: bool) -> Result<Token, LexError> {
        self.bump_char();
        let mut body = String::new();
        let mut in_class = false;
        let mut escaped = false;

        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.error_at("unterminated regexp literal", Span::point(start)));
            };

            if is_line_terminator(ch) {
                return Err(
                    self.error_at("unterminated regexp literal", Span::point(self.position()))
                );
            }

            body.push(ch);
            self.bump_char();

            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '[' => in_class = true,
                ']' if in_class => in_class = false,
                '/' if !in_class => {
                    body.pop();
                    break;
                }
                _ => {}
            }
        }

        let mut flags = String::new();
        while let Some(ch) = self.peek_char() {
            if !is_identifier_part(ch) {
                break;
            }
            flags.push(ch);
            self.bump_char();
        }

        Ok(self.make_token(TokenKind::RegExp { body, flags }, start, leading_line_break))
    }

    fn read_identifier_char(&mut self, is_start: bool) -> Result<char, LexError> {
        if self.peek_char() == Some('\\') {
            return self.read_identifier_escape(is_start);
        }

        let start = self.position();
        let Some(ch) = self.bump_char() else {
            return Err(self.error_at("identifier expected", Span::point(start)));
        };
        let valid = if is_start {
            is_identifier_start(ch)
        } else {
            is_identifier_part(ch)
        };
        if !valid {
            return Err(self.error_at("invalid identifier character", Span::point(start)));
        }
        Ok(ch)
    }

    fn read_identifier_escape(&mut self, is_start: bool) -> Result<char, LexError> {
        let start = self.position();
        self.bump_char();
        if self.peek_char() != Some('u') {
            return Err(self.error_at(
                "invalid unicode escape sequence",
                Span::point(self.position()),
            ));
        }
        self.bump_char();

        let value = self.read_unicode_escape_value(start)?;
        let ch = char::from_u32(value)
            .ok_or_else(|| self.error_at("invalid unicode code point", Span::point(start)))?;
        let valid = if is_start {
            is_identifier_start(ch)
        } else {
            is_identifier_part(ch)
        };
        if !valid {
            return Err(self.error_at("invalid escaped identifier character", Span::point(start)));
        }
        Ok(ch)
    }

    fn lex_private_name(
        &mut self,
        start: Position,
        leading_line_break: bool,
    ) -> Result<Token, LexError> {
        self.bump_char();
        let Some(ch) = self.peek_char() else {
            return Err(self.error_at("expected private name", Span::point(start)));
        };
        if ch != '\\' && !is_identifier_start(ch) {
            return Err(self.error_at(
                "invalid first character of private name",
                Span::point(self.position()),
            ));
        }

        let mut name = String::new();
        let mut escaped = self.peek_char() == Some('\\');
        name.push(self.read_identifier_char(true)?);
        while let Some(ch) = self.peek_char() {
            if ch == '\\' {
                escaped = true;
                name.push(self.read_identifier_char(false)?);
                continue;
            }
            if !is_identifier_part(ch) {
                break;
            }
            name.push(ch);
            self.bump_char();
        }

        Ok(self.make_token_with_escape(
            TokenKind::PrivateName(name),
            start,
            leading_line_break,
            escaped,
        ))
    }

    fn lex_identifier(
        &mut self,
        start: Position,
        leading_line_break: bool,
    ) -> Result<Token, LexError> {
        let mut name = String::new();
        let mut escaped = self.peek_char() == Some('\\');
        name.push(self.read_identifier_char(true)?);

        while let Some(ch) = self.peek_char() {
            if ch == '\\' {
                escaped = true;
                name.push(self.read_identifier_char(false)?);
                continue;
            }
            if !is_identifier_part(ch) {
                break;
            }
            name.push(ch);
            self.bump_char();
        }

        let kind = if escaped {
            TokenKind::Identifier(name)
        } else {
            keyword_kind(&name).unwrap_or(TokenKind::Identifier(name))
        };
        Ok(self.make_token_with_escape(kind, start, leading_line_break, escaped))
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<bool, LexError> {
        let mut saw_line_break = false;

        loop {
            if self.offset == 0 && self.starts_with("#!") {
                self.consume_str("#!");
                while let Some(ch) = self.peek_char() {
                    if is_line_terminator(ch) {
                        break;
                    }
                    self.bump_char();
                }
                continue;
            }

            if self.starts_with("<!--") {
                if !self.allow_html_comments {
                    return Err(self.error_at(
                        "HTML-like comments are not allowed in module code",
                        Span::point(self.position()),
                    ));
                }
                self.consume_str("<!--");
                while let Some(ch) = self.peek_char() {
                    if is_line_terminator(ch) {
                        break;
                    }
                    self.bump_char();
                }
                continue;
            }

            if self.starts_with("-->") && self.html_close_comment_allowed {
                if !self.allow_html_comments {
                    return Err(self.error_at(
                        "HTML-like comments are not allowed in module code",
                        Span::point(self.position()),
                    ));
                }
                self.consume_str("-->");
                while let Some(ch) = self.peek_char() {
                    if is_line_terminator(ch) {
                        break;
                    }
                    self.bump_char();
                }
                continue;
            }

            if self.starts_with("//") {
                self.consume_str("//");
                while let Some(ch) = self.peek_char() {
                    if is_line_terminator(ch) {
                        break;
                    }
                    self.bump_char();
                }
                continue;
            }

            if self.starts_with("/*") {
                self.consume_str("/*");
                let mut terminated = false;
                while let Some(ch) = self.peek_char() {
                    if self.starts_with("*/") {
                        self.consume_str("*/");
                        terminated = true;
                        break;
                    }
                    if is_line_terminator(ch) {
                        saw_line_break = true;
                        self.consume_line_break();
                        self.html_close_comment_allowed = true;
                    } else {
                        self.bump_char();
                    }
                }
                if !terminated {
                    return Err(
                        self.error_at("unterminated block comment", Span::point(self.position()))
                    );
                }
                continue;
            }

            match self.peek_char() {
                Some(ch) if is_line_terminator(ch) => {
                    saw_line_break = true;
                    self.consume_line_break();
                    self.html_close_comment_allowed = true;
                }
                Some(ch) if is_whitespace(ch) => {
                    self.bump_char();
                }
                _ => break,
            }
        }

        Ok(saw_line_break)
    }

    fn read_escape_sequence(&mut self) -> Result<Option<String>, LexError> {
        let start = self.position();
        let Some(ch) = self.peek_char() else {
            return Err(self.error_at("unterminated escape sequence", Span::point(start)));
        };

        let value = match ch {
            '\'' => {
                self.bump_char();
                "'".to_string()
            }
            '"' => {
                self.bump_char();
                "\"".to_string()
            }
            '`' => {
                self.bump_char();
                "`".to_string()
            }
            '\\' => {
                self.bump_char();
                "\\".to_string()
            }
            'n' => {
                self.bump_char();
                "\n".to_string()
            }
            'r' => {
                self.bump_char();
                "\r".to_string()
            }
            't' => {
                self.bump_char();
                "\t".to_string()
            }
            'b' => {
                self.bump_char();
                "\u{0008}".to_string()
            }
            'f' => {
                self.bump_char();
                "\u{000c}".to_string()
            }
            'v' => {
                self.bump_char();
                "\u{000b}".to_string()
            }
            '0' => {
                self.bump_char();
                "\0".to_string()
            }
            'x' => {
                self.bump_char();
                let code = self.read_fixed_hex_digits(2)?;
                codepoint_to_string(code, Span::point(start))?
            }
            'u' => {
                self.bump_char();
                let code = self.read_unicode_escape_value(start)?;
                codepoint_to_string(code, Span::point(start))?
            }
            ch if is_line_terminator(ch) => {
                self.consume_line_break();
                return Ok(None);
            }
            _ => {
                self.bump_char();
                ch.to_string()
            }
        };

        Ok(Some(value))
    }

    fn read_template_escape_sequence(&mut self) -> Result<(Option<String>, bool), LexError> {
        let start = self.position();
        let Some(ch) = self.peek_char() else {
            return Err(self.error_at("unterminated escape sequence", Span::point(start)));
        };

        let result = match ch {
            '\'' => {
                self.bump_char();
                (Some("'".to_string()), false)
            }
            '"' => {
                self.bump_char();
                (Some("\"".to_string()), false)
            }
            '`' => {
                self.bump_char();
                (Some("`".to_string()), false)
            }
            '\\' => {
                self.bump_char();
                (Some("\\".to_string()), false)
            }
            'n' => {
                self.bump_char();
                (Some("\n".to_string()), false)
            }
            'r' => {
                self.bump_char();
                (Some("\r".to_string()), false)
            }
            't' => {
                self.bump_char();
                (Some("\t".to_string()), false)
            }
            'b' => {
                self.bump_char();
                (Some("\u{0008}".to_string()), false)
            }
            'f' => {
                self.bump_char();
                (Some("\u{000c}".to_string()), false)
            }
            'v' => {
                self.bump_char();
                (Some("\u{000b}".to_string()), false)
            }
            '0' => {
                self.bump_char();
                let invalid = self.peek_char().is_some_and(|next| next.is_ascii_digit());
                (Some("\0".to_string()), invalid)
            }
            'x' => {
                self.bump_char();
                let checkpoint = (self.offset, self.line, self.column);
                match self
                    .read_fixed_hex_digits(2)
                    .and_then(|code| codepoint_to_string(code, Span::point(start)))
                {
                    Ok(value) => (Some(value), false),
                    Err(_) => {
                        (self.offset, self.line, self.column) = checkpoint;
                        (Some("x".to_string()), true)
                    }
                }
            }
            'u' => {
                self.bump_char();
                let checkpoint = (self.offset, self.line, self.column);
                match self
                    .read_unicode_escape_value(start)
                    .and_then(|code| codepoint_to_string(code, Span::point(start)))
                {
                    Ok(value) => (Some(value), false),
                    Err(_) => {
                        (self.offset, self.line, self.column) = checkpoint;
                        (Some("u".to_string()), true)
                    }
                }
            }
            c if is_line_terminator(c) => {
                self.consume_line_break();
                return Ok((None, false));
            }
            _ => {
                self.bump_char();
                (Some(ch.to_string()), ch.is_ascii_digit())
            }
        };

        Ok(result)
    }

    fn read_fixed_hex_digits(&mut self, count: usize) -> Result<u32, LexError> {
        let start = self.position();
        let mut value = 0u32;
        for _ in 0..count {
            let Some(ch) = self.peek_char() else {
                return Err(
                    self.error_at("invalid hexadecimal escape sequence", Span::point(start))
                );
            };
            let Some(digit) = ch.to_digit(16) else {
                return Err(
                    self.error_at("invalid hexadecimal escape sequence", Span::point(start))
                );
            };
            value = value * 16 + digit;
            self.bump_char();
        }
        Ok(value)
    }

    fn read_unicode_escape_value(&mut self, start: Position) -> Result<u32, LexError> {
        if self.peek_char() == Some('{') {
            self.bump_char();
            let mut value = 0u32;
            let mut digits = 0usize;
            let mut terminated = false;
            while let Some(ch) = self.peek_char() {
                if ch == '}' {
                    self.bump_char();
                    terminated = true;
                    break;
                }
                let Some(digit) = ch.to_digit(16) else {
                    return Err(self.error_at(
                        "invalid unicode escape sequence",
                        Span::point(self.position()),
                    ));
                };
                value = value.saturating_mul(16).saturating_add(digit);
                digits += 1;
                self.bump_char();
            }
            if digits == 0 || !terminated {
                return Err(self.error_at("invalid unicode escape sequence", Span::point(start)));
            }
            Ok(value)
        } else {
            self.read_fixed_hex_digits(4)
        }
    }

    fn validate_numeric_literal(&self, raw: &str, start: Position) -> Result<(), LexError> {
        let (numeric, bigint) = raw
            .strip_suffix('n')
            .map_or((raw, false), |value| (value, true));

        if numeric.starts_with("0x")
            || numeric.starts_with("0X")
            || numeric.starts_with("0o")
            || numeric.starts_with("0O")
            || numeric.starts_with("0b")
            || numeric.starts_with("0B")
        {
            self.validate_numeric_separator_run(&numeric[2..], start)?;
            return Ok(());
        }

        if bigint && (numeric.contains('.') || numeric.contains('e') || numeric.contains('E')) {
            return Err(self.error_at("invalid bigint literal", Span::point(start)));
        }

        let (mantissa, exponent) = match numeric.find(['e', 'E']) {
            Some(index) => (&numeric[..index], Some(&numeric[index + 1..])),
            None => (numeric, None),
        };

        if let Some(exponent) = exponent {
            let exponent = exponent
                .strip_prefix('+')
                .or_else(|| exponent.strip_prefix('-'))
                .unwrap_or(exponent);
            self.validate_numeric_separator_run(exponent, start)?;
        }

        if let Some((integer, fraction)) = mantissa.split_once('.') {
            if !integer.is_empty() {
                self.validate_numeric_separator_run(integer, start)?;
            }
            if !fraction.is_empty() {
                self.validate_numeric_separator_run(fraction, start)?;
            }
        } else {
            self.validate_numeric_separator_run(mantissa, start)?;
        }

        if bigint && numeric.len() > 1 && numeric.starts_with('0') {
            return Err(self.error_at("invalid bigint literal", Span::point(start)));
        }

        if numeric.contains('_')
            && numeric.starts_with('0')
            && numeric.len() > 1
            && !numeric.starts_with("0.")
        {
            return Err(self.error_at("invalid numeric separator placement", Span::point(start)));
        }

        Ok(())
    }

    fn validate_numeric_separator_run(
        &self,
        digits: &str,
        start: Position,
    ) -> Result<(), LexError> {
        if digits.is_empty()
            || digits.starts_with('_')
            || digits.ends_with('_')
            || digits.contains("__")
        {
            return Err(self.error_at("invalid numeric separator placement", Span::point(start)));
        }
        Ok(())
    }

    fn consume_decimal_digits(&mut self, raw: &mut String) -> usize {
        let mut count = 0;
        while let Some(ch) = self.peek_char() {
            if ch == '_' {
                raw.push(ch);
                self.bump_char();
                continue;
            }
            if !ch.is_ascii_digit() {
                break;
            }
            raw.push(ch);
            self.bump_char();
            count += 1;
        }
        count
    }

    fn consume_radix_digits(&mut self, raw: &mut String, radix: u32) -> usize {
        let mut count = 0;
        while let Some(ch) = self.peek_char() {
            if ch == '_' {
                raw.push(ch);
                self.bump_char();
                continue;
            }
            let Some(_) = ch.to_digit(radix) else {
                break;
            };
            raw.push(ch);
            self.bump_char();
            count += 1;
        }
        count
    }

    fn simple_token(
        &mut self,
        kind: TokenKind,
        start: Position,
        leading_line_break: bool,
    ) -> Token {
        self.bump_char();
        self.make_token(kind, start, leading_line_break)
    }

    fn make_token(&self, kind: TokenKind, start: Position, leading_line_break: bool) -> Token {
        self.make_token_with_escape(kind, start, leading_line_break, false)
    }

    fn make_token_with_escape(
        &self,
        kind: TokenKind,
        start: Position,
        leading_line_break: bool,
        escaped: bool,
    ) -> Token {
        let tag = kind.tag();
        Token {
            tag,
            kind,
            span: Span::new(start, self.position()),
            leading_line_break,
            escaped,
        }
    }

    fn error_at(&self, message: impl Into<String>, span: Span) -> LexError {
        LexError::new(message, span)
    }

    fn position(&self) -> Position {
        Position::new(self.offset, self.line, self.column)
    }

    fn regex_allowed(&self) -> bool {
        let Some(previous) = self.last_token else {
            return true;
        };

        !matches!(
            previous,
            TokenTag::Identifier
                | TokenTag::PrivateName
                | TokenTag::Number
                | TokenTag::String
                | TokenTag::Template
                | TokenTag::Null
                | TokenTag::False
                | TokenTag::True
                | TokenTag::This
                | TokenTag::Super
                | TokenTag::RightParen
                | TokenTag::RightBracket
                | TokenTag::Increment
                | TokenTag::Decrement
        )
    }

    fn prefer_division_over_regex(&self) -> bool {
        let Some(previous) = self.last_token else {
            return false;
        };

        match previous {
            TokenTag::RightBrace => {
                self.previous_source_char()
                    .is_some_and(|ch| is_whitespace(ch) || is_line_terminator(ch))
                    && self
                        .next_non_whitespace_after_slash()
                        .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
            }
            TokenTag::Yield => !self.regexp_terminates_before_statement_end(),
            _ => false,
        }
    }

    fn regexp_terminates_before_statement_end(&self) -> bool {
        let mut escaped = false;
        let mut in_class = false;

        for ch in self.source[self.offset + 1..].chars() {
            if is_line_terminator(ch) || ch == ';' {
                return false;
            }
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '[' => in_class = true,
                ']' if in_class => in_class = false,
                '/' if !in_class => return true,
                _ => {}
            }
        }

        false
    }

    fn previous_source_char(&self) -> Option<char> {
        self.source[..self.offset].chars().next_back()
    }

    fn next_non_whitespace_after_slash(&self) -> Option<char> {
        self.source[self.offset + 1..]
            .chars()
            .find(|ch| !is_whitespace(*ch) && !is_line_terminator(*ch))
    }

    fn looks_like_identifier_escape(&self) -> bool {
        self.starts_with("\\u")
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn peek_nth_char(&self, n: usize) -> Option<char> {
        self.source[self.offset..].chars().nth(n)
    }

    fn starts_with(&self, text: &str) -> bool {
        self.source[self.offset..].starts_with(text)
    }

    fn consume_str(&mut self, text: &str) {
        for _ in text.chars() {
            self.bump_char();
        }
    }

    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        self.column += 1;
        Some(ch)
    }

    fn consume_line_break(&mut self) {
        if self.starts_with("\r\n") {
            self.offset += 2;
        } else if let Some(ch) = self.peek_char() {
            self.offset += ch.len_utf8();
        }
        self.line += 1;
        self.column = 1;
    }
}

fn is_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}'
            | '\u{000b}'
            | '\u{000c}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{feff}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn is_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
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

fn keyword_kind(name: &str) -> Option<TokenKind> {
    Some(match name {
        "null" => TokenKind::Null,
        "false" => TokenKind::False,
        "true" => TokenKind::True,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "return" => TokenKind::Return,
        "var" => TokenKind::Var,
        "this" => TokenKind::This,
        "delete" => TokenKind::Delete,
        "void" => TokenKind::Void,
        "typeof" => TokenKind::Typeof,
        "new" => TokenKind::New,
        "in" => TokenKind::In,
        "instanceof" => TokenKind::Instanceof,
        "do" => TokenKind::Do,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "switch" => TokenKind::Switch,
        "case" => TokenKind::Case,
        "default" => TokenKind::Default,
        "throw" => TokenKind::Throw,
        "try" => TokenKind::Try,
        "catch" => TokenKind::Catch,
        "finally" => TokenKind::Finally,
        "function" => TokenKind::Function,
        "debugger" => TokenKind::Debugger,
        "with" => TokenKind::With,
        "class" => TokenKind::Class,
        "const" => TokenKind::Const,
        "enum" => TokenKind::Enum,
        "export" => TokenKind::Export,
        "extends" => TokenKind::Extends,
        "import" => TokenKind::Import,
        "super" => TokenKind::Super,
        "implements" => TokenKind::Implements,
        "interface" => TokenKind::Interface,
        "let" => TokenKind::Let,
        "package" => TokenKind::Package,
        "private" => TokenKind::Private,
        "protected" => TokenKind::Protected,
        "public" => TokenKind::Public,
        "static" => TokenKind::Static,
        "yield" => TokenKind::Yield,
        "await" => TokenKind::Await,
        _ => return None,
    })
}

fn codepoint_to_string(value: u32, span: Span) -> Result<String, LexError> {
    if value <= 0xffff && (0xd800..=0xdfff).contains(&value) {
        return Ok(String::from_utf16_lossy(&[value as u16]));
    }

    let ch =
        char::from_u32(value).ok_or_else(|| LexError::new("invalid unicode code point", span))?;
    Ok(ch.to_string())
}
