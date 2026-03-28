use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
};

use crate::{
    ast::*,
    lexer::{LexError, Lexer},
    regexp::parse_regexp_pattern,
    token::{Span, Token, TokenKind, TokenTag},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl ParseError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {} column {}",
            self.message, self.span.start.line, self.span.start.column
        )
    }
}

impl Error for ParseError {}

impl From<LexError> for ParseError {
    fn from(value: LexError) -> Self {
        Self::new(value.message, value.span)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseOptions {
    pub is_module: bool,
    pub is_strict: bool,
}

pub struct Parser {
    source: String,
    tokens: Vec<Token>,
    index: usize,
    is_module: bool,
    is_strict: bool,
    function_depth: usize,
    await_context: bool,
    yield_context: bool,
    allow_let_binding_identifier: bool,
    allow_let_identifier: bool,
    escaped_identifiers: HashSet<Span>,
    parenthesized_expressions: HashSet<Span>,
}

impl Parser {
    pub fn new(source: &str) -> Result<Self, ParseError> {
        Self::new_with_options(source, ParseOptions::default())
    }

    pub fn new_with_options(source: &str, options: ParseOptions) -> Result<Self, ParseError> {
        let tokens = Lexer::new_with_html_comments(source, !options.is_module).scan_all()?;
        Ok(Self::from_tokens_with_source_and_options(
            tokens,
            source.to_string(),
            options,
        ))
    }

    pub fn from_tokens(tokens: Vec<Token>) -> Self {
        Self::from_tokens_with_options(tokens, ParseOptions::default())
    }

    pub fn from_tokens_with_options(tokens: Vec<Token>, options: ParseOptions) -> Self {
        Self::from_tokens_with_source_and_options(tokens, String::new(), options)
    }

    fn from_tokens_with_source_and_options(
        tokens: Vec<Token>,
        source: String,
        options: ParseOptions,
    ) -> Self {
        let escaped_identifiers = tokens
            .iter()
            .filter(|token| token.escaped && token.kind.identifier().is_some())
            .map(|token| token.span)
            .collect();
        Self {
            source,
            tokens,
            index: 0,
            is_module: options.is_module,
            is_strict: options.is_module || options.is_strict,
            function_depth: 0,
            await_context: options.is_module,
            yield_context: false,
            allow_let_binding_identifier: false,
            allow_let_identifier: true,
            escaped_identifiers,
            parenthesized_expressions: HashSet::new(),
        }
    }

    pub fn parse(source: &str) -> Result<Program, ParseError> {
        Self::new(source)?.parse_program()
    }

    pub fn parse_with_options(source: &str, options: ParseOptions) -> Result<Program, ParseError> {
        Self::new_with_options(source, options)?.parse_program()
    }

    pub fn parse_program(&mut self) -> Result<Program, ParseError> {
        let start = self.current().span;
        let mut body = Vec::new();
        let mut allow_directives = true;

        while !self.at(TokenTag::Eof) {
            if allow_directives {
                if let Some(directive) = self.try_parse_directive_statement()? {
                    body.push(directive);
                    continue;
                }
                allow_directives = false;
            }

            body.push(self.parse_statement()?);
        }

        let end = body.last().map_or(self.current().span, Statement::span);
        let program = Program {
            body,
            span: Span::between(start, end),
        };
        self.validate_program(&program)?;
        Ok(program)
    }

    fn try_parse_directive_statement(&mut self) -> Result<Option<Statement>, ParseError> {
        let value = match &self.current().kind {
            TokenKind::String(value) => value.clone(),
            _ => return Ok(None),
        };

        let next = self.peek(1);
        let ends_statement = next.is_none_or(|token| {
            matches!(
                token.tag(),
                TokenTag::Semicolon | TokenTag::RightBrace | TokenTag::Eof
            ) || token.leading_line_break
        });
        if !ends_statement {
            return Ok(None);
        }

        let token = self.advance();
        self.consume_semicolon()?;
        Ok(Some(Statement::Directive(Directive {
            value: value.clone(),
            span: token.span,
        })))
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        if self.at_async_function() {
            return Ok(Statement::FunctionDeclaration(
                self.parse_function_like(true, true)?,
            ));
        }
        if self.at(TokenTag::At) {
            return Ok(Statement::ClassDeclaration(self.parse_class(true)?));
        }
        if self.at_binding_identifier() && self.peek_tag(1) == Some(TokenTag::Colon) {
            return Ok(Statement::Labeled(self.parse_labeled_statement()?));
        }
        if self.at_await_using_declaration_start() || self.at_using_declaration_start() {
            let declaration = self.parse_variable_declaration(true, true)?;
            self.consume_semicolon()?;
            return Ok(Statement::VariableDeclaration(declaration));
        }
        if self.at(TokenTag::Import)
            && matches!(self.peek_tag(1), Some(TokenTag::LeftParen | TokenTag::Dot))
        {
            return Ok(Statement::Expression(self.parse_expression_statement()?));
        }

        match self.current_tag() {
            TokenTag::Semicolon => {
                let token = self.advance();
                Ok(Statement::Empty(token.span))
            }
            TokenTag::LeftBrace => Ok(Statement::Block(self.parse_block_statement()?)),
            TokenTag::Import => Ok(Statement::ImportDeclaration(
                self.parse_import_declaration()?,
            )),
            TokenTag::Export => Ok(Statement::ExportDeclaration(
                self.parse_export_declaration()?,
            )),
            TokenTag::Function => Ok(Statement::FunctionDeclaration(
                self.parse_function_like(true, false)?,
            )),
            TokenTag::Class => Ok(Statement::ClassDeclaration(self.parse_class(true)?)),
            TokenTag::Var | TokenTag::Const => {
                let declaration = self.parse_variable_declaration(true, true)?;
                self.consume_semicolon()?;
                Ok(Statement::VariableDeclaration(declaration))
            }
            TokenTag::Let if self.at_let_declaration_start() => {
                let declaration = self.parse_variable_declaration(true, true)?;
                self.consume_semicolon()?;
                Ok(Statement::VariableDeclaration(declaration))
            }
            TokenTag::If => Ok(Statement::If(self.parse_if_statement()?)),
            TokenTag::While => Ok(Statement::While(self.parse_while_statement()?)),
            TokenTag::Do => Ok(Statement::DoWhile(self.parse_do_while_statement()?)),
            TokenTag::For => Ok(Statement::For(self.parse_for_statement()?)),
            TokenTag::Switch => Ok(Statement::Switch(self.parse_switch_statement()?)),
            TokenTag::With => Ok(Statement::With(self.parse_with_statement()?)),
            TokenTag::Return => Ok(Statement::Return(self.parse_return_statement()?)),
            TokenTag::Break => Ok(Statement::Break(self.parse_jump_statement("break")?)),
            TokenTag::Continue => Ok(Statement::Continue(self.parse_jump_statement("continue")?)),
            TokenTag::Throw => Ok(Statement::Throw(self.parse_throw_statement()?)),
            TokenTag::Try => Ok(Statement::Try(self.parse_try_statement()?)),
            TokenTag::Debugger => {
                let span = self.advance_span();
                self.consume_semicolon()?;
                Ok(Statement::Debugger(span))
            }
            TokenTag::Let => self.with_let_identifier_allowed(|parser| {
                Ok(Statement::Expression(parser.parse_expression_statement()?))
            }),
            _ => Ok(Statement::Expression(self.parse_expression_statement()?)),
        }
    }

    fn parse_block_statement(&mut self) -> Result<BlockStatement, ParseError> {
        let start = self.expect_span(TokenTag::LeftBrace)?;
        let mut body = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            body.push(self.parse_statement()?);
        }

        let end = self.expect_span(TokenTag::RightBrace)?;
        Ok(BlockStatement {
            body,
            span: Span::between(start, end),
        })
    }

    fn parse_expression_statement(&mut self) -> Result<ExpressionStatement, ParseError> {
        let expression = self.parse_expression(true)?;
        let span = expression.span();
        self.consume_semicolon()?;
        Ok(ExpressionStatement { expression, span })
    }

    fn parse_labeled_statement(&mut self) -> Result<LabeledStatement, ParseError> {
        let label = self.parse_binding_identifier()?;
        self.expect(TokenTag::Colon)?;
        let body = Box::new(self.parse_statement()?);
        let span = Span::between(label.span, body.span());
        Ok(LabeledStatement { label, body, span })
    }

    fn parse_import_declaration(&mut self) -> Result<ImportDeclaration, ParseError> {
        let start = self.expect_span(TokenTag::Import)?;

        if let TokenKind::String(value) = &self.current().kind {
            let value = value.clone();
            let source = StringLiteral {
                value,
                span: self.advance_span(),
            };
            let attributes = self.parse_module_attributes()?;
            self.consume_semicolon()?;
            let end = attributes.as_ref().map_or(source.span, Expression::span);
            return Ok(ImportDeclaration {
                clause: None,
                source,
                attributes,
                is_defer: false,
                span: Span::between(start, end),
            });
        }

        let mut is_defer = false;
        let default = if self.current_is_identifier_named("defer")
            && self.peek_tag(1) == Some(TokenTag::Mul)
        {
            self.advance();
            is_defer = true;
            None
        } else if self.at_binding_identifier() {
            Some(self.parse_binding_identifier()?)
        } else {
            None
        };

        let clause = if default.is_some() && self.eat(TokenTag::Comma) {
            if self.at(TokenTag::Mul) {
                Some(ImportClause::Namespace {
                    default,
                    namespace: self.parse_namespace_import()?,
                })
            } else if self.at(TokenTag::LeftBrace) {
                Some(ImportClause::Named {
                    default,
                    specifiers: self.parse_import_specifiers()?,
                })
            } else {
                return Err(self.error_current("import clause expected after ','"));
            }
        } else if self.at(TokenTag::Mul) {
            let namespace = self.parse_namespace_import()?;
            Some(ImportClause::Namespace { default, namespace })
        } else if self.at(TokenTag::LeftBrace) {
            Some(ImportClause::Named {
                default,
                specifiers: self.parse_import_specifiers()?,
            })
        } else if let Some(default) = default {
            Some(ImportClause::Default(default))
        } else {
            return Err(self.error_current("import clause expected"));
        };

        self.expect_contextual("from")?;
        let source = self.parse_module_specifier()?;
        let attributes = self.parse_module_attributes()?;
        self.consume_semicolon()?;
        let end = attributes.as_ref().map_or(source.span, Expression::span);
        Ok(ImportDeclaration {
            clause,
            source,
            attributes,
            is_defer,
            span: Span::between(start, end),
        })
    }

    fn parse_export_declaration(&mut self) -> Result<ExportDeclaration, ParseError> {
        let start = self.expect_span(TokenTag::Export)?;

        if self.eat(TokenTag::Default) {
            let declaration = if self.at_async_function() {
                ExportDefaultKind::Function(self.parse_function_like(false, true)?)
            } else if self.at(TokenTag::Function) {
                ExportDefaultKind::Function(self.parse_function_like(false, false)?)
            } else if self.at(TokenTag::Class) {
                ExportDefaultKind::Class(self.parse_class(false)?)
            } else {
                let expression = self.parse_assignment_expression(true)?;
                self.consume_semicolon()?;
                ExportDefaultKind::Expression(expression)
            };
            let end = match &declaration {
                ExportDefaultKind::Function(node) => node.span,
                ExportDefaultKind::Class(node) => node.span,
                ExportDefaultKind::Expression(node) => node.span(),
            };
            return Ok(ExportDeclaration::Default(ExportDefaultDeclaration {
                declaration,
                span: Span::between(start, end),
            }));
        }

        if self.eat(TokenTag::Mul) {
            let exported = if self.eat_contextual("as") {
                Some(self.parse_module_export_name()?)
            } else {
                None
            };
            self.expect_contextual("from")?;
            let source = self.parse_module_specifier()?;
            let attributes = self.parse_module_attributes()?;
            self.consume_semicolon()?;
            let end = attributes.as_ref().map_or(source.span, Expression::span);
            return Ok(ExportDeclaration::All(ExportAllDeclaration {
                exported,
                source,
                attributes,
                span: Span::between(start, end),
            }));
        }

        if self.at(TokenTag::LeftBrace) {
            let specifiers = self.parse_export_specifiers()?;
            let source = if self.current_is_identifier_named("from") {
                self.advance();
                Some(self.parse_module_specifier()?)
            } else {
                None
            };
            let attributes = self.parse_module_attributes()?;
            self.consume_semicolon()?;
            let end = attributes
                .as_ref()
                .map(Expression::span)
                .or_else(|| source.as_ref().map(|item| item.span))
                .or_else(|| specifiers.last().map(|item| item.span))
                .unwrap_or(start);
            return Ok(ExportDeclaration::Named(ExportNamedDeclaration {
                specifiers,
                source,
                attributes,
                span: Span::between(start, end),
            }));
        }

        let declaration = if self.at_async_function() {
            ExportedDeclaration::Function(self.parse_function_like(true, true)?)
        } else if self.at(TokenTag::Function) {
            ExportedDeclaration::Function(self.parse_function_like(true, false)?)
        } else if self.at(TokenTag::Class) {
            ExportedDeclaration::Class(self.parse_class(true)?)
        } else if matches!(
            self.current_tag(),
            TokenTag::Var | TokenTag::Let | TokenTag::Const
        ) {
            let declaration = self.parse_variable_declaration(true, true)?;
            self.consume_semicolon()?;
            ExportedDeclaration::Variable(declaration)
        } else {
            return Err(self.error_current("export declaration expected"));
        };

        Ok(ExportDeclaration::Declaration(declaration))
    }

    fn parse_if_statement(&mut self) -> Result<IfStatement, ParseError> {
        let start = self.expect_span(TokenTag::If)?;
        let test = self.parse_parenthesized_expression()?;
        let consequent = Box::new(self.parse_statement()?);
        let alternate = if self.eat(TokenTag::Else) {
            Some(Box::new(self.parse_statement()?))
        } else {
            None
        };
        let end = alternate
            .as_ref()
            .map_or_else(|| consequent.span(), |node| node.span());
        Ok(IfStatement {
            test,
            consequent,
            alternate,
            span: Span::between(start, end),
        })
    }

    fn parse_while_statement(&mut self) -> Result<WhileStatement, ParseError> {
        let start = self.expect_span(TokenTag::While)?;
        let test = self.parse_parenthesized_expression()?;
        let body = Box::new(self.parse_statement()?);
        let end = body.span();
        Ok(WhileStatement {
            test,
            body,
            span: Span::between(start, end),
        })
    }

    fn parse_do_while_statement(&mut self) -> Result<DoWhileStatement, ParseError> {
        let start = self.expect_span(TokenTag::Do)?;
        let body = Box::new(self.parse_statement()?);
        self.expect(TokenTag::While)?;
        let test = self.parse_parenthesized_expression()?;
        let end = test.span();
        self.eat(TokenTag::Semicolon);
        Ok(DoWhileStatement {
            body,
            test,
            span: Span::between(start, end),
        })
    }

    fn parse_with_statement(&mut self) -> Result<WithStatement, ParseError> {
        let start = self.expect_span(TokenTag::With)?;
        let object = self.parse_parenthesized_expression()?;
        let body = Box::new(self.parse_statement()?);
        let end = body.span();
        Ok(WithStatement {
            object,
            body,
            span: Span::between(start, end),
        })
    }

    fn parse_switch_statement(&mut self) -> Result<SwitchStatement, ParseError> {
        let start = self.expect_span(TokenTag::Switch)?;
        let discriminant = self.parse_parenthesized_expression()?;
        self.expect(TokenTag::LeftBrace)?;
        let mut cases = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            let case_start = self.current().span;
            let test = if self.eat(TokenTag::Case) {
                Some(self.parse_expression(true)?)
            } else if self.eat(TokenTag::Default) {
                None
            } else {
                return Err(self.error_current("expected case or default"));
            };
            self.expect(TokenTag::Colon)?;

            let mut consequent = Vec::new();
            while !matches!(
                self.current_tag(),
                TokenTag::Case | TokenTag::Default | TokenTag::RightBrace | TokenTag::Eof
            ) {
                consequent.push(self.parse_statement()?);
            }

            let end = consequent
                .last()
                .map_or(self.previous().span, Statement::span);
            cases.push(SwitchCase {
                test,
                consequent,
                span: Span::between(case_start, end),
            });
        }

        let end = self.expect_span(TokenTag::RightBrace)?;
        Ok(SwitchStatement {
            discriminant,
            cases,
            span: Span::between(start, end),
        })
    }

    fn parse_namespace_import(&mut self) -> Result<Identifier, ParseError> {
        self.expect(TokenTag::Mul)?;
        self.expect_contextual("as")?;
        self.parse_binding_identifier()
    }

    fn parse_import_specifiers(&mut self) -> Result<Vec<ImportSpecifier>, ParseError> {
        self.expect(TokenTag::LeftBrace)?;
        let mut specifiers = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            let imported = self.parse_module_export_name()?;
            let local = if self.eat_contextual("as") {
                self.parse_binding_identifier()?
            } else if let ModuleExportName::Identifier(identifier) = imported.clone() {
                identifier
            } else {
                return Err(self.error_current("string import names require an alias"));
            };
            let span = Span::between(imported.span(), local.span);
            specifiers.push(ImportSpecifier {
                imported,
                local,
                span,
            });

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        self.expect(TokenTag::RightBrace)?;
        Ok(specifiers)
    }

    fn parse_export_specifiers(&mut self) -> Result<Vec<ExportSpecifier>, ParseError> {
        self.expect(TokenTag::LeftBrace)?;
        let mut specifiers = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            let local = self.parse_module_export_name()?;
            let exported = if self.eat_contextual("as") {
                self.parse_module_export_name()?
            } else {
                local.clone()
            };
            let span = Span::between(local.span(), exported.span());
            specifiers.push(ExportSpecifier {
                local,
                exported,
                span,
            });

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        self.expect(TokenTag::RightBrace)?;
        Ok(specifiers)
    }

    fn parse_module_specifier(&mut self) -> Result<StringLiteral, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::String(value) => Ok(StringLiteral {
                value,
                span: token.span,
            }),
            _ => Err(ParseError::new("module specifier expected", token.span)),
        }
    }

    fn parse_module_export_name(&mut self) -> Result<ModuleExportName, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::String(value) => {
                if self
                    .source_slice(token.span)
                    .is_some_and(|raw| !self.raw_string_is_well_formed_unicode(raw))
                {
                    return Err(ParseError::new(
                        "module export names must be well-formed Unicode strings",
                        token.span,
                    ));
                }
                Ok(ModuleExportName::String(StringLiteral {
                    value,
                    span: token.span,
                }))
            }
            TokenKind::Identifier(name) => Ok(ModuleExportName::Identifier(Identifier {
                name,
                span: token.span,
            })),
            TokenKind::Yield if !self.yield_context => {
                Ok(ModuleExportName::Identifier(Identifier {
                    name: "yield".to_string(),
                    span: token.span,
                }))
            }
            TokenKind::Await if !self.await_context => {
                Ok(ModuleExportName::Identifier(Identifier {
                    name: "await".to_string(),
                    span: token.span,
                }))
            }
            kind => {
                if let Some(name) = kind.keyword_text() {
                    Ok(ModuleExportName::Identifier(Identifier {
                        name: name.to_string(),
                        span: token.span,
                    }))
                } else {
                    Err(ParseError::new("module export name expected", token.span))
                }
            }
        }
    }

    fn parse_module_attributes(&mut self) -> Result<Option<Expression>, ParseError> {
        if !self.at(TokenTag::With) {
            return Ok(None);
        }

        self.advance();
        if !self.at(TokenTag::LeftBrace) {
            return Err(self.error_current("expected import attributes object"));
        }
        self.parse_object_expression().map(Some)
    }

    fn parse_for_statement(&mut self) -> Result<ForStatement, ParseError> {
        let start = self.expect_span(TokenTag::For)?;
        let is_await = self.await_context && self.eat(TokenTag::Await);
        self.expect(TokenTag::LeftParen)?;

        if self.eat(TokenTag::Semicolon) {
            if is_await {
                return Err(self.error_current("for await requires an of clause"));
            }
            let test = if self.at(TokenTag::Semicolon) {
                None
            } else {
                Some(self.parse_expression(true)?)
            };
            self.expect(TokenTag::Semicolon)?;
            let update = if self.at(TokenTag::RightParen) {
                None
            } else {
                Some(self.parse_expression(true)?)
            };
            self.expect(TokenTag::RightParen)?;
            let body = Box::new(self.parse_statement()?);
            let end = body.span();
            return Ok(ForStatement::Classic(ForClassicStatement {
                init: None,
                test,
                update,
                body,
                span: Span::between(start, end),
            }));
        }

        if self.at_for_head_declaration_start() {
            let declaration = self.parse_variable_declaration(false, false)?;
            if self.current_is_contextual_of() || self.at(TokenTag::In) {
                let is_of = self.current_is_contextual_of();
                if declaration.declarations.len() != 1 {
                    return Err(self.error_current(
                        "for-in/of declarations must contain exactly one declarator",
                    ));
                }
                if declaration
                    .declarations
                    .iter()
                    .any(|item| item.init.is_some())
                    && !(self.at(TokenTag::In)
                        && declaration.kind == VariableKind::Var
                        && declaration
                            .declarations
                            .iter()
                            .all(|item| matches!(item.pattern, Pattern::Identifier(_))))
                {
                    return Err(
                        self.error_current("for-in/of declarations do not support an initializer")
                    );
                }
                if self.at(TokenTag::In)
                    && matches!(
                        declaration.kind,
                        VariableKind::Using | VariableKind::AwaitUsing
                    )
                {
                    return Err(self.error_current("using declarations require an of clause"));
                }
                self.advance();
                let right = self.parse_expression(true)?;
                self.expect(TokenTag::RightParen)?;
                let body = Box::new(self.parse_statement()?);
                let end = body.span();
                let left = ForLeft::VariableDeclaration(declaration);
                let span = Span::between(start, end);
                return Ok(if is_of {
                    ForStatement::Of(ForEachStatement {
                        left,
                        right,
                        is_await,
                        body,
                        span,
                    })
                } else {
                    if is_await {
                        return Err(self.error_current("for await requires an of clause"));
                    }
                    ForStatement::In(ForEachStatement {
                        left,
                        right,
                        is_await: false,
                        body,
                        span,
                    })
                });
            }

            if matches!(
                declaration.kind,
                VariableKind::Const | VariableKind::Using | VariableKind::AwaitUsing
            ) && declaration
                .declarations
                .iter()
                .any(|item| item.init.is_none())
            {
                return Err(self.error_current("const declarations require an initializer"));
            }

            if is_await {
                return Err(self.error_current("for await requires an of clause"));
            }

            self.expect(TokenTag::Semicolon)?;
            let test = if self.at(TokenTag::Semicolon) {
                None
            } else {
                Some(self.parse_expression(true)?)
            };
            self.expect(TokenTag::Semicolon)?;
            let update = if self.at(TokenTag::RightParen) {
                None
            } else {
                Some(self.parse_expression(true)?)
            };
            self.expect(TokenTag::RightParen)?;
            let body = Box::new(self.parse_statement()?);
            let end = body.span();
            return Ok(ForStatement::Classic(ForClassicStatement {
                init: Some(ForInit::VariableDeclaration(declaration)),
                test,
                update,
                body,
                span: Span::between(start, end),
            }));
        }

        let init = if self.at(TokenTag::Let) {
            self.with_let_identifier_allowed(|parser| {
                if matches!(
                    parser.current_tag(),
                    TokenTag::LeftBrace | TokenTag::LeftBracket
                ) && !parser.at_assignment_pattern_start()
                {
                    parser.parse_assignment_target_expression()
                } else {
                    parser.parse_expression(false)
                }
            })?
        } else if matches!(
            self.current_tag(),
            TokenTag::LeftBrace | TokenTag::LeftBracket
        ) && !self.at_assignment_pattern_start()
        {
            self.parse_assignment_target_expression()?
        } else {
            self.parse_expression(false)?
        };
        if self.current_is_contextual_of() || self.at(TokenTag::In) {
            if self.current_is_contextual_of()
                && matches!(init, Expression::Identifier(Identifier { ref name, .. }) if name == "let")
            {
                return Err(self.error_current("for-of left side cannot start with let"));
            }
            if !self.expression_is_for_in_of_left(&init) {
                return Err(
                    self.error_current("for-in/of left side must be a left-hand-side expression")
                );
            }
            let is_of = self.current_is_contextual_of();
            self.advance();
            let right = self.parse_expression(true)?;
            self.expect(TokenTag::RightParen)?;
            let body = Box::new(self.parse_statement()?);
            let end = body.span();
            let left = ForLeft::Expression(init);
            let span = Span::between(start, end);
            return Ok(if is_of {
                ForStatement::Of(ForEachStatement {
                    left,
                    right,
                    is_await,
                    body,
                    span,
                })
            } else {
                if is_await {
                    return Err(self.error_current("for await requires an of clause"));
                }
                ForStatement::In(ForEachStatement {
                    left,
                    right,
                    is_await: false,
                    body,
                    span,
                })
            });
        }

        if is_await {
            return Err(self.error_current("for await requires an of clause"));
        }

        self.expect(TokenTag::Semicolon)?;
        let test = if self.at(TokenTag::Semicolon) {
            None
        } else {
            Some(self.parse_expression(true)?)
        };
        self.expect(TokenTag::Semicolon)?;
        let update = if self.at(TokenTag::RightParen) {
            None
        } else {
            Some(self.parse_expression(true)?)
        };
        self.expect(TokenTag::RightParen)?;
        let body = Box::new(self.parse_statement()?);
        let end = body.span();
        Ok(ForStatement::Classic(ForClassicStatement {
            init: Some(ForInit::Expression(init)),
            test,
            update,
            body,
            span: Span::between(start, end),
        }))
    }

    fn parse_return_statement(&mut self) -> Result<ReturnStatement, ParseError> {
        let start = self.expect_span(TokenTag::Return)?;
        if self.function_depth == 0 {
            return Err(ParseError::new("return not in a function", start));
        }

        let argument = if self.statement_is_terminated_here() {
            None
        } else {
            Some(self.parse_expression(true)?)
        };
        let end = argument.as_ref().map_or(start, Expression::span);
        self.consume_semicolon()?;
        Ok(ReturnStatement {
            argument,
            span: Span::between(start, end),
        })
    }

    fn parse_jump_statement(&mut self, _keyword: &str) -> Result<JumpStatement, ParseError> {
        let start = self.advance_span();
        let label = if self.statement_is_terminated_here() {
            None
        } else {
            Some(self.parse_binding_identifier()?)
        };
        let end = label.as_ref().map_or(start, |node| node.span);
        self.consume_semicolon()?;
        Ok(JumpStatement {
            label,
            span: Span::between(start, end),
        })
    }

    fn parse_throw_statement(&mut self) -> Result<ThrowStatement, ParseError> {
        let start = self.expect_span(TokenTag::Throw)?;
        if self.current().leading_line_break {
            return Err(ParseError::new(
                "line terminator not allowed after throw",
                start,
            ));
        }
        let argument = self.parse_expression(true)?;
        let span = Span::between(start, argument.span());
        self.consume_semicolon()?;
        Ok(ThrowStatement { argument, span })
    }

    fn parse_try_statement(&mut self) -> Result<TryStatement, ParseError> {
        let start = self.expect_span(TokenTag::Try)?;
        let block = self.parse_block_statement()?;
        let handler = if self.eat(TokenTag::Catch) {
            let catch_start = self.previous().span;
            let param = if self.eat(TokenTag::LeftParen) {
                let pattern = self.parse_binding_pattern_atom()?;
                self.expect(TokenTag::RightParen)?;
                Some(pattern)
            } else {
                None
            };
            let body = self.parse_block_statement()?;
            Some(CatchClause {
                param,
                span: Span::between(catch_start, body.span),
                body,
            })
        } else {
            None
        };
        let finalizer = if self.eat(TokenTag::Finally) {
            Some(self.parse_block_statement()?)
        } else {
            None
        };

        if handler.is_none() && finalizer.is_none() {
            return Err(self.error_current("expecting catch or finally"));
        }

        let end = finalizer
            .as_ref()
            .map(|block| block.span)
            .or_else(|| handler.as_ref().map(|clause| clause.span))
            .unwrap_or(block.span);

        Ok(TryStatement {
            block,
            handler,
            finalizer,
            span: Span::between(start, end),
        })
    }

    fn parse_parenthesized_expression(&mut self) -> Result<Expression, ParseError> {
        self.expect(TokenTag::LeftParen)?;
        let expression = self.parse_expression(true)?;
        self.expect(TokenTag::RightParen)?;
        self.parenthesized_expressions.insert(expression.span());
        Ok(expression)
    }

    fn parse_variable_declaration(
        &mut self,
        allow_in: bool,
        require_initializer: bool,
    ) -> Result<VariableDeclaration, ParseError> {
        let (kind, keyword_span) = self.parse_variable_declaration_head()?;

        let mut declarations = Vec::new();
        loop {
            let pattern = if matches!(kind, VariableKind::Using | VariableKind::AwaitUsing) {
                Pattern::Identifier(self.parse_binding_identifier()?)
            } else if kind == VariableKind::Var && self.at(TokenTag::Let) {
                Pattern::Identifier(Identifier {
                    name: "let".to_string(),
                    span: self.advance_span(),
                })
            } else {
                self.parse_binding_pattern_atom()?
            };
            let init = if self.eat(TokenTag::Assign) {
                Some(self.parse_assignment_expression(allow_in)?)
            } else {
                None
            };

            if require_initializer
                && matches!(
                    kind,
                    VariableKind::Const | VariableKind::Using | VariableKind::AwaitUsing
                )
                && init.is_none()
            {
                return Err(self.error_current("const declarations require an initializer"));
            }

            let span = init.as_ref().map_or_else(
                || pattern.span(),
                |expr| Span::between(pattern.span(), expr.span()),
            );
            declarations.push(VariableDeclarator {
                pattern,
                init,
                span,
            });

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = declarations
            .last()
            .map(|node| node.span)
            .unwrap_or(keyword_span);
        Ok(VariableDeclaration {
            kind,
            declarations,
            span: Span::between(keyword_span, end),
        })
    }

    fn parse_variable_declaration_head(&mut self) -> Result<(VariableKind, Span), ParseError> {
        if matches!(
            self.current_tag(),
            TokenTag::Var | TokenTag::Let | TokenTag::Const
        ) {
            let keyword = self.advance();
            let kind = match keyword.tag() {
                TokenTag::Var => VariableKind::Var,
                TokenTag::Let => VariableKind::Let,
                TokenTag::Const => VariableKind::Const,
                _ => unreachable!(),
            };
            return Ok((kind, keyword.span));
        }

        if self.at_using_declaration_start() {
            let keyword = self.advance();
            return Ok((VariableKind::Using, keyword.span));
        }

        if self.at_await_using_declaration_start() {
            let await_span = self.expect_span(TokenTag::Await)?;
            let using_span = self.expect_contextual("using")?.span;
            return Ok((
                VariableKind::AwaitUsing,
                Span::between(await_span, using_span),
            ));
        }

        Err(ParseError::new(
            "expected variable declaration",
            self.current().span,
        ))
    }

    fn parse_binding_pattern(&mut self) -> Result<Pattern, ParseError> {
        let base = self.parse_binding_pattern_atom()?;
        if self.eat(TokenTag::Assign) {
            let right = self.parse_assignment_expression(true)?;
            let span = Span::between(base.span(), right.span());
            return Ok(Pattern::Assignment(AssignmentPattern {
                left: Box::new(base),
                right,
                span,
            }));
        }
        Ok(base)
    }

    fn parse_binding_pattern_atom(&mut self) -> Result<Pattern, ParseError> {
        match self.current_tag() {
            _ if self.at_binding_identifier() => {
                Ok(Pattern::Identifier(self.parse_binding_identifier()?))
            }
            TokenTag::LeftBracket => self.parse_array_pattern(),
            TokenTag::LeftBrace => self.parse_object_pattern(),
            _ => Err(self.error_current("expected binding pattern")),
        }
    }

    fn parse_array_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start = self.expect_span(TokenTag::LeftBracket)?;
        let mut elements = Vec::new();

        while !self.at(TokenTag::RightBracket) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Comma) {
                elements.push(None);
                continue;
            }

            if self.eat(TokenTag::Ellipsis) {
                let rest_start = self.previous().span;
                let argument = self.parse_binding_pattern_atom()?;
                let span = Span::between(rest_start, argument.span());
                elements.push(Some(Pattern::Rest(RestPattern {
                    argument: Box::new(argument),
                    span,
                })));
                if self.eat(TokenTag::Comma) {
                    return Err(self.error_current("rest element must be the last one"));
                }
                break;
            }

            elements.push(Some(self.parse_binding_pattern()?));
            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = self.expect_span(TokenTag::RightBracket)?;
        Ok(Pattern::Array(ArrayPattern {
            elements,
            span: Span::between(start, end),
        }))
    }

    fn parse_object_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start = self.expect_span(TokenTag::LeftBrace)?;
        let mut properties = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Ellipsis) {
                let rest_start = self.previous().span;
                let argument = self.parse_binding_pattern_atom()?;
                let span = Span::between(rest_start, argument.span());
                properties.push(ObjectPatternProperty::Rest {
                    argument: Box::new(argument),
                    span,
                });
                if self.eat(TokenTag::Comma) {
                    return Err(self.error_current("rest property must be the last one"));
                }
                break;
            }

            let key = self.parse_property_key()?;
            let (mut value, shorthand) = if self.eat(TokenTag::Colon) {
                (self.parse_binding_pattern()?, false)
            } else if let PropertyKey::Identifier(identifier) = key.clone() {
                self.ensure_identifier_name_allowed(&identifier.name, identifier.span)?;
                (Pattern::Identifier(identifier), true)
            } else {
                return Err(self.error_current("object pattern property requires ':'"));
            };

            if self.eat(TokenTag::Assign) {
                let right = self.parse_assignment_expression(true)?;
                let span = Span::between(value.span(), right.span());
                value = Pattern::Assignment(AssignmentPattern {
                    left: Box::new(value),
                    right,
                    span,
                });
            }

            let span = Span::between(key.span(), value.span());
            properties.push(ObjectPatternProperty::Property {
                key,
                value: Box::new(value),
                shorthand,
                span,
            });

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = self.expect_span(TokenTag::RightBrace)?;
        Ok(Pattern::Object(ObjectPattern {
            properties,
            span: Span::between(start, end),
        }))
    }

    fn parse_function_like(
        &mut self,
        require_name: bool,
        is_async: bool,
    ) -> Result<Function, ParseError> {
        let start = if is_async {
            self.advance_span()
        } else {
            self.current().span
        };
        self.expect(TokenTag::Function)?;
        let is_generator = self.eat(TokenTag::Mul);
        let id = if require_name {
            if self.at_binding_identifier() {
                Some(self.parse_binding_identifier()?)
            } else {
                return Err(self.error_current("function name expected"));
            }
        } else if self.at_function_expression_name() {
            Some(self.parse_function_expression_name()?)
        } else {
            None
        };
        let (params, body) = self.with_function_context(is_async, is_generator, |parser| {
            let params = parser.parse_parameter_list()?;
            let body = parser.parse_block_statement()?;
            Ok((params, body))
        })?;
        let span = Span::between(start, body.span);

        Ok(Function {
            id,
            params,
            body,
            is_async,
            is_generator,
            span,
        })
    }

    fn parse_function_expression_name(&mut self) -> Result<Identifier, ParseError> {
        let name = match &self.current().kind {
            TokenKind::Identifier(name) => name.clone(),
            TokenKind::Yield => "yield".to_string(),
            TokenKind::Await => "await".to_string(),
            TokenKind::Let => "let".to_string(),
            kind => self
                .relaxed_keyword_identifier_name(kind)
                .map(ToString::to_string)
                .ok_or_else(|| ParseError::new("identifier expected", self.current().span))?,
        };
        let span = self.advance_span();
        Ok(Identifier { name, span })
    }

    fn parse_method_function(
        &mut self,
        start: Span,
        is_async: bool,
        is_generator: bool,
    ) -> Result<Function, ParseError> {
        let (params, body) = self.with_function_context(is_async, is_generator, |parser| {
            let params = parser.parse_parameter_list()?;
            let body = parser.parse_block_statement()?;
            Ok((params, body))
        })?;
        let span = Span::between(start, body.span);

        Ok(Function {
            id: None,
            params,
            body,
            is_async,
            is_generator,
            span,
        })
    }

    fn parse_class(&mut self, require_name: bool) -> Result<Class, ParseError> {
        let decorators = self.parse_decorator_list()?;
        let start = decorators
            .first()
            .map(Expression::span)
            .unwrap_or(self.current().span);
        self.expect(TokenTag::Class)?;
        let id = if self.at_binding_identifier() {
            Some(self.parse_binding_identifier()?)
        } else if require_name {
            return Err(self.error_current("class statement requires a name"));
        } else {
            None
        };

        let super_class = if self.eat(TokenTag::Extends) {
            Some(self.parse_left_hand_side_expression()?)
        } else {
            None
        };

        self.expect(TokenTag::LeftBrace)?;
        let mut body = Vec::new();
        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Semicolon) {
                body.push(ClassElement::Empty(self.previous().span));
                continue;
            }

            let decorators = self.parse_decorator_list()?;
            if self.at_static_block_start() {
                if !decorators.is_empty() {
                    return Err(self.error_current("decorators are not allowed on static blocks"));
                }
                self.advance();
                body.push(ClassElement::StaticBlock(self.parse_block_statement()?));
                continue;
            }

            body.push(self.parse_class_element(decorators)?);
        }
        let end = self.expect_span(TokenTag::RightBrace)?;

        Ok(Class {
            decorators,
            id,
            super_class,
            body,
            span: Span::between(start, end),
        })
    }

    fn parse_class_element(
        &mut self,
        decorators: Vec<Expression>,
    ) -> Result<ClassElement, ParseError> {
        let start = decorators
            .first()
            .map(Expression::span)
            .unwrap_or(self.current().span);
        let is_static = if self.at_class_static_modifier() {
            self.advance();
            true
        } else {
            false
        };

        if self.at_class_accessor_start() {
            let accessor_name = self.advance();
            let kind = match accessor_name.kind.identifier() {
                Some("get") => MethodKind::Getter,
                Some("set") => MethodKind::Setter,
                _ => unreachable!(),
            };
            let key = self.parse_property_key_with_private(true)?;
            let value = self.parse_method_function(start, false, false)?;
            let span = Span::between(start, value.span);
            return Ok(ClassElement::Method(ClassMethod {
                decorators,
                key,
                value,
                kind,
                is_static,
                span,
            }));
        }

        if self.at_auto_accessor_field_start() {
            self.advance();
            let key = self.parse_property_key_with_private(true)?;
            let value = if self.eat(TokenTag::Assign) {
                Some(self.with_field_initializer_context(|parser| {
                    parser.parse_assignment_expression(true)
                })?)
            } else {
                None
            };
            let end = value.as_ref().map_or(key.span(), Expression::span);
            self.consume_class_element_terminator()?;
            return Ok(ClassElement::Field(ClassField {
                decorators,
                key,
                value,
                is_static,
                is_accessor: true,
                span: Span::between(start, end),
            }));
        }

        let (is_async, is_generator) = if self.at_async_generator_method_prefix(true) {
            self.advance();
            self.expect(TokenTag::Mul)?;
            (true, true)
        } else if self.at_async_method_prefix(true) {
            self.advance();
            (true, false)
        } else {
            (false, self.eat(TokenTag::Mul))
        };
        let key = self.parse_property_key_with_private(true)?;

        if self.at(TokenTag::LeftParen) {
            let value = self.parse_method_function(start, is_async, is_generator)?;
            let kind = if !is_static && !is_generator && !is_async && self.is_constructor_key(&key)
            {
                MethodKind::Constructor
            } else {
                MethodKind::Method
            };
            let span = Span::between(start, value.span);
            return Ok(ClassElement::Method(ClassMethod {
                decorators,
                key,
                value,
                kind,
                is_static,
                span,
            }));
        }

        let value = if self.eat(TokenTag::Assign) {
            Some(self.with_field_initializer_context(|parser| {
                parser.parse_assignment_expression(true)
            })?)
        } else {
            None
        };
        let end = value.as_ref().map_or(key.span(), Expression::span);
        self.consume_class_element_terminator()?;
        Ok(ClassElement::Field(ClassField {
            decorators,
            key,
            value,
            is_static,
            is_accessor: false,
            span: Span::between(start, end),
        }))
    }

    fn parse_parameter_list(&mut self) -> Result<Vec<Pattern>, ParseError> {
        self.with_let_binding_identifier_allowed(|parser| {
            parser.expect(TokenTag::LeftParen)?;
            let mut params = Vec::new();

            while !parser.at(TokenTag::RightParen) && !parser.at(TokenTag::Eof) {
                if parser.eat(TokenTag::Ellipsis) {
                    let rest_start = parser.previous().span;
                    let argument = parser.parse_binding_pattern_atom()?;
                    let span = Span::between(rest_start, argument.span());
                    params.push(Pattern::Rest(RestPattern {
                        argument: Box::new(argument),
                        span,
                    }));
                    if parser.eat(TokenTag::Comma) {
                        return Err(parser.error_current("rest parameter must be the last one"));
                    }
                    break;
                }

                params.push(parser.parse_binding_pattern()?);
                if !parser.eat(TokenTag::Comma) {
                    break;
                }
            }

            parser.expect(TokenTag::RightParen)?;
            Ok(params)
        })
    }

    fn parse_arrow_function(&mut self, is_async: bool) -> Result<Expression, ParseError> {
        let start = if is_async {
            self.advance_span()
        } else {
            self.current().span
        };

        let (params, body) = self.with_function_context(is_async, false, |parser| {
            let params =
                if parser.at_binding_identifier() && parser.peek_tag(1) == Some(TokenTag::Arrow) {
                    vec![Pattern::Identifier(parser.parse_binding_identifier()?)]
                } else {
                    parser.parse_parameter_list()?
                };

            parser.expect(TokenTag::Arrow)?;
            let body = if parser.at(TokenTag::LeftBrace) {
                ArrowBody::Block(parser.parse_block_statement()?)
            } else {
                ArrowBody::Expression(Box::new(parser.parse_assignment_expression(true)?))
            };
            Ok((params, body))
        })?;
        let end = match &body {
            ArrowBody::Expression(expression) => expression.span(),
            ArrowBody::Block(block) => block.span,
        };

        Ok(Expression::ArrowFunction(Box::new(ArrowFunction {
            params,
            body,
            is_async,
            span: Span::between(start, end),
        })))
    }

    fn parse_expression(&mut self, _allow_in: bool) -> Result<Expression, ParseError> {
        let allow_in = _allow_in;
        let first = self.parse_assignment_expression(allow_in)?;
        if !self.eat(TokenTag::Comma) {
            return Ok(first);
        }

        let mut expressions = vec![first];
        loop {
            expressions.push(self.parse_assignment_expression(allow_in)?);
            if !self.eat(TokenTag::Comma) {
                break;
            }
        }
        let span = Span::between(expressions[0].span(), expressions.last().unwrap().span());
        Ok(Expression::Sequence(SequenceExpression {
            expressions,
            span,
        }))
    }

    fn parse_assignment_expression(&mut self, allow_in: bool) -> Result<Expression, ParseError> {
        if self.at_async_arrow_start() {
            return self.parse_arrow_function(true);
        }
        if self.at_arrow_start() {
            return self.parse_arrow_function(false);
        }
        if self.at(TokenTag::Yield) && self.yield_context {
            return self.parse_yield_expression(allow_in);
        }
        if self.at_assignment_pattern_start() {
            let left = self.parse_assignment_target_expression()?;
            self.expect(TokenTag::Assign)?;
            let right = self.parse_assignment_expression(allow_in)?;
            let span = Span::between(left.span(), right.span());
            return Ok(Expression::Assignment(Box::new(AssignmentExpression {
                operator: AssignmentOperator::Assign,
                left,
                right,
                span,
            })));
        }

        let left = self.parse_expression_bp(0, allow_in)?;
        if let Some(operator) = self.current_assignment_operator() {
            self.advance();
            let right = self.parse_assignment_expression(allow_in)?;
            let span = Span::between(left.span(), right.span());
            return Ok(Expression::Assignment(Box::new(AssignmentExpression {
                operator,
                left,
                right,
                span,
            })));
        }
        Ok(left)
    }

    fn parse_yield_expression(&mut self, allow_in: bool) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::Yield)?;
        let delegate = !self.current().leading_line_break && self.eat(TokenTag::Mul);
        let argument = if delegate || self.yield_has_argument() {
            Some(self.parse_assignment_expression(allow_in)?)
        } else {
            None
        };
        let end = argument.as_ref().map_or_else(
            || {
                if delegate {
                    self.previous().span
                } else {
                    start
                }
            },
            Expression::span,
        );
        Ok(Expression::Yield(Box::new(YieldExpression {
            argument,
            delegate,
            span: Span::between(start, end),
        })))
    }

    fn parse_expression_bp(
        &mut self,
        min_bp: u8,
        allow_in: bool,
    ) -> Result<Expression, ParseError> {
        let mut left = self.parse_unary_expression()?;

        loop {
            if self.at(TokenTag::Question) {
                let conditional_bp = 0;
                if conditional_bp < min_bp {
                    break;
                }

                self.advance();
                let consequent = self.parse_assignment_expression(true)?;
                self.expect(TokenTag::Colon)?;
                let alternate = self.parse_assignment_expression(allow_in)?;
                let span = Span::between(left.span(), alternate.span());
                left = Expression::Conditional(Box::new(ConditionalExpression {
                    test: left,
                    consequent,
                    alternate,
                    span,
                }));
                continue;
            }

            let Some((operator, left_bp, right_bp)) = self.current_infix_operator(allow_in) else {
                break;
            };
            if left_bp < min_bp {
                break;
            }

            self.advance();
            let right = self.parse_expression_bp(right_bp, allow_in)?;
            let span = Span::between(left.span(), right.span());
            left = match operator {
                InfixOperator::Binary(operator) => Expression::Binary(Box::new(BinaryExpression {
                    operator,
                    left,
                    right,
                    span,
                })),
                InfixOperator::Logical(operator) => {
                    Expression::Logical(Box::new(LogicalExpression {
                        operator,
                        left,
                        right,
                        span,
                    }))
                }
            };
        }

        Ok(left)
    }

    fn parse_unary_expression(&mut self) -> Result<Expression, ParseError> {
        if self.at(TokenTag::Await) && self.await_context {
            return self.parse_await_expression();
        }
        if self.at(TokenTag::PrivateName) && self.peek_tag(1) == Some(TokenTag::In) {
            let token = self.advance();
            let TokenKind::PrivateName(name) = token.kind else {
                unreachable!();
            };
            return Ok(Expression::PrivateIdentifier(Identifier {
                name,
                span: token.span,
            }));
        }

        let start = self.current().span;
        let operator = match self.current_tag() {
            TokenTag::Delete => Some(UnaryOperator::Delete),
            TokenTag::Void => Some(UnaryOperator::Void),
            TokenTag::Typeof => Some(UnaryOperator::Typeof),
            TokenTag::Add => Some(UnaryOperator::Positive),
            TokenTag::Sub => Some(UnaryOperator::Negative),
            TokenTag::Not => Some(UnaryOperator::LogicalNot),
            TokenTag::BitNot => Some(UnaryOperator::BitNot),
            TokenTag::Increment => {
                self.advance();
                let argument = self.parse_unary_expression()?;
                let span = Span::between(start, argument.span());
                return Ok(Expression::Update(Box::new(UpdateExpression {
                    operator: UpdateOperator::Increment,
                    argument,
                    prefix: true,
                    span,
                })));
            }
            TokenTag::Decrement => {
                self.advance();
                let argument = self.parse_unary_expression()?;
                let span = Span::between(start, argument.span());
                return Ok(Expression::Update(Box::new(UpdateExpression {
                    operator: UpdateOperator::Decrement,
                    argument,
                    prefix: true,
                    span,
                })));
            }
            _ => None,
        };

        if let Some(operator) = operator {
            self.advance();
            let argument = self.parse_unary_expression()?;
            let span = Span::between(start, argument.span());
            return Ok(Expression::Unary(Box::new(UnaryExpression {
                operator,
                argument,
                span,
            })));
        }

        let expression = self.parse_left_hand_side_expression()?;
        if !self.current().leading_line_break {
            match self.current_tag() {
                TokenTag::Increment => {
                    let end = self.advance_span();
                    return Ok(Expression::Update(Box::new(UpdateExpression {
                        operator: UpdateOperator::Increment,
                        argument: expression,
                        prefix: false,
                        span: Span::between(start, end),
                    })));
                }
                TokenTag::Decrement => {
                    let end = self.advance_span();
                    return Ok(Expression::Update(Box::new(UpdateExpression {
                        operator: UpdateOperator::Decrement,
                        argument: expression,
                        prefix: false,
                        span: Span::between(start, end),
                    })));
                }
                _ => {}
            }
        }
        Ok(expression)
    }

    fn parse_await_expression(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::Await)?;
        let argument = self.parse_unary_expression()?;
        let span = Span::between(start, argument.span());
        Ok(Expression::Await(Box::new(AwaitExpression {
            argument,
            span,
        })))
    }

    fn parse_left_hand_side_expression(&mut self) -> Result<Expression, ParseError> {
        if self.at(TokenTag::New) {
            return self.parse_new_expression();
        }

        let primary = self.parse_primary_expression()?;
        if matches!(primary, Expression::Super(_)) {
            return self.parse_super_expression(primary);
        }
        self.parse_chain_expression(primary)
    }

    fn parse_super_expression(&mut self, expression: Expression) -> Result<Expression, ParseError> {
        let start = expression.span();
        let expression = match self.current_tag() {
            TokenTag::Dot => self.parse_named_member_suffix(expression, false)?,
            TokenTag::LeftBracket => self.parse_computed_member_suffix(expression, false)?,
            TokenTag::LeftParen => {
                let arguments = self.parse_arguments()?;
                let end = arguments.last().map_or(start, CallArgument::span);
                Expression::Call(Box::new(CallExpression {
                    callee: expression,
                    arguments,
                    optional: false,
                    span: Span::between(start, end),
                }))
            }
            _ => {
                return Err(ParseError::new(
                    "super can only be used in a member access or call",
                    start,
                ));
            }
        };
        self.parse_chain_expression(expression)
    }

    fn parse_new_expression(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::New)?;
        if self.eat(TokenTag::Dot) {
            let property = self.parse_binding_identifier()?;
            let span = Span::between(start, property.span);
            let expression = Expression::MetaProperty(Box::new(MetaPropertyExpression {
                meta: Identifier {
                    name: "new".to_string(),
                    span: start,
                },
                property,
                span,
            }));
            return self.parse_chain_expression(expression);
        }

        let callee = if self.at(TokenTag::New) {
            self.parse_new_expression()?
        } else {
            let primary = self.parse_primary_expression()?;
            self.parse_member_suffix_chain(primary)?
        };

        let arguments = if self.at(TokenTag::LeftParen) {
            self.parse_arguments()?
        } else {
            Vec::new()
        };

        let end = arguments.last().map_or(callee.span(), CallArgument::span);
        let expression = Expression::New(Box::new(NewExpression {
            callee,
            arguments,
            span: Span::between(start, end),
        }));
        self.parse_chain_expression(expression)
    }

    fn parse_primary_expression(&mut self) -> Result<Expression, ParseError> {
        if self.at(TokenTag::At) {
            let class = self.parse_class(false)?;
            return Ok(Expression::Class(Box::new(class)));
        }
        if self.at_async_function() {
            let function = self.parse_function_like(false, true)?;
            return Ok(Expression::Function(Box::new(function)));
        }

        let allow_yield_identifier = !self.yield_context;
        let allow_await_identifier = !self.await_context;
        let token = self.advance();
        match token.kind {
            TokenKind::Identifier(name) => {
                if token.escaped
                    && self.escaped_identifier_is_reserved_here(
                        &name,
                        self.is_strict,
                        !allow_await_identifier,
                        !allow_yield_identifier,
                    )
                {
                    return Err(ParseError::new(
                        format!("escaped reserved word '{name}' is not allowed here"),
                        token.span,
                    ));
                }
                if token.escaped
                    && name == "import"
                    && matches!(self.current_tag(), TokenTag::LeftParen | TokenTag::Dot)
                {
                    return Err(ParseError::new(
                        "escaped 'import' is not allowed here",
                        token.span,
                    ));
                }
                Ok(Expression::Identifier(Identifier {
                    name,
                    span: token.span,
                }))
            }
            TokenKind::Yield if allow_yield_identifier => Ok(Expression::Identifier(Identifier {
                name: "yield".to_string(),
                span: token.span,
            })),
            TokenKind::Await if allow_await_identifier => Ok(Expression::Identifier(Identifier {
                name: "await".to_string(),
                span: token.span,
            })),
            TokenKind::Let if self.allow_let_identifier => Ok(Expression::Identifier(Identifier {
                name: "let".to_string(),
                span: token.span,
            })),
            kind if self.relaxed_keyword_identifier_name(&kind).is_some() => {
                Ok(Expression::Identifier(Identifier {
                    name: self
                        .relaxed_keyword_identifier_name(&kind)
                        .expect("checked above")
                        .to_string(),
                    span: token.span,
                }))
            }
            TokenKind::Number(raw) => Ok(Expression::Literal(Literal::Number(NumberLiteral {
                raw,
                span: token.span,
            }))),
            TokenKind::String(value) => Ok(Expression::Literal(Literal::String(StringLiteral {
                value,
                span: token.span,
            }))),
            TokenKind::Template {
                value,
                invalid_escape,
            } => {
                if invalid_escape {
                    return Err(ParseError::new(
                        "invalid escape sequence in template literal",
                        token.span,
                    ));
                }
                Ok(Expression::Literal(Literal::Template(TemplateLiteral {
                    value,
                    span: token.span,
                })))
            }
            TokenKind::RegExp { body, flags } => {
                let pattern = parse_regexp_pattern(&body, &flags)
                    .map_err(|error| ParseError::new(error.message, token.span))?;
                Ok(Expression::Literal(Literal::RegExp(RegExpLiteral {
                    body,
                    flags,
                    pattern,
                    span: token.span,
                })))
            }
            TokenKind::Null => Ok(Expression::Literal(Literal::Null(token.span))),
            TokenKind::True => Ok(Expression::Literal(Literal::Boolean(BooleanLiteral {
                value: true,
                span: token.span,
            }))),
            TokenKind::False => Ok(Expression::Literal(Literal::Boolean(BooleanLiteral {
                value: false,
                span: token.span,
            }))),
            TokenKind::This => Ok(Expression::This(token.span)),
            TokenKind::Super => Ok(Expression::Super(token.span)),
            TokenKind::Function => {
                self.rewind_one();
                let function = self.parse_function_like(false, false)?;
                Ok(Expression::Function(Box::new(function)))
            }
            TokenKind::Class => {
                self.rewind_one();
                let class = self.parse_class(false)?;
                Ok(Expression::Class(Box::new(class)))
            }
            TokenKind::Import
                if matches!(self.current_tag(), TokenTag::LeftParen | TokenTag::Dot) =>
            {
                if token.escaped {
                    return Err(ParseError::new(
                        "escaped 'import' is not allowed here",
                        token.span,
                    ));
                }
                Ok(Expression::Identifier(Identifier {
                    name: "import".to_string(),
                    span: token.span,
                }))
            }
            TokenKind::LeftParen => {
                self.rewind_one();
                self.parse_parenthesized_expression()
            }
            TokenKind::LeftBracket => {
                self.rewind_one();
                self.parse_array_expression()
            }
            TokenKind::LeftBrace => {
                self.rewind_one();
                self.parse_object_expression()
            }
            other => Err(ParseError::new(
                format!("unexpected token '{other}' in expression"),
                token.span,
            )),
        }
    }

    fn parse_array_expression(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::LeftBracket)?;
        let mut elements = Vec::new();

        while !self.at(TokenTag::RightBracket) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Comma) {
                elements.push(None);
                continue;
            }

            if self.eat(TokenTag::Ellipsis) {
                let spread_start = self.previous().span;
                let argument = self.parse_assignment_expression(true)?;
                let span = Span::between(spread_start, argument.span());
                elements.push(Some(ArrayElement::Spread { argument, span }));
            } else {
                elements.push(Some(ArrayElement::Expression(
                    self.parse_assignment_expression(true)?,
                )));
            }

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = self.expect_span(TokenTag::RightBracket)?;
        Ok(Expression::Array(ArrayExpression {
            elements,
            span: Span::between(start, end),
        }))
    }

    fn parse_object_expression(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::LeftBrace)?;
        let mut properties = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Ellipsis) {
                let spread_start = self.previous().span;
                let argument = self.parse_assignment_expression(true)?;
                let span = Span::between(spread_start, argument.span());
                properties.push(ObjectProperty::Spread { argument, span });
            } else {
                let property_start = self.current().span;
                let (key, value, shorthand, kind) = if self.at_object_method_start() {
                    self.parse_object_method_property()?
                } else {
                    let key = self.parse_property_key()?;
                    let (value, shorthand) = if self.eat(TokenTag::Colon) {
                        (self.parse_assignment_expression(true)?, false)
                    } else if let PropertyKey::Identifier(identifier) = key.clone() {
                        (Expression::Identifier(identifier), true)
                    } else {
                        return Err(self.error_current("object literal property requires ':'"));
                    };
                    (key, value, shorthand, ObjectPropertyKind::Init)
                };

                let span = Span::between(property_start, value.span());
                properties.push(ObjectProperty::Property {
                    key,
                    value,
                    shorthand,
                    kind,
                    span,
                });
            }

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = self.expect_span(TokenTag::RightBrace)?;
        Ok(Expression::Object(ObjectExpression {
            properties,
            span: Span::between(start, end),
        }))
    }

    fn parse_assignment_target_expression(&mut self) -> Result<Expression, ParseError> {
        match self.current_tag() {
            TokenTag::LeftBracket | TokenTag::LeftBrace
                if self.assignment_target_uses_left_hand_side_expression() =>
            {
                self.parse_left_hand_side_expression()
            }
            TokenTag::LeftBracket => self.parse_array_assignment_pattern_expression(),
            TokenTag::LeftBrace => self.parse_object_assignment_pattern_expression(),
            _ => self.parse_left_hand_side_expression(),
        }
    }

    fn parse_assignment_target_with_default(&mut self) -> Result<Expression, ParseError> {
        let left = self.parse_assignment_target_expression()?;
        if self.eat(TokenTag::Assign) {
            let right = self.parse_assignment_expression(true)?;
            let span = Span::between(left.span(), right.span());
            Ok(Expression::Assignment(Box::new(AssignmentExpression {
                operator: AssignmentOperator::Assign,
                left,
                right,
                span,
            })))
        } else {
            Ok(left)
        }
    }

    fn parse_array_assignment_pattern_expression(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::LeftBracket)?;
        let mut elements = Vec::new();

        while !self.at(TokenTag::RightBracket) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Comma) {
                elements.push(None);
                continue;
            }

            if self.eat(TokenTag::Ellipsis) {
                let spread_start = self.previous().span;
                let argument = self.parse_assignment_target_expression()?;
                let span = Span::between(spread_start, argument.span());
                elements.push(Some(ArrayElement::Spread { argument, span }));
                if self.eat(TokenTag::Comma) {
                    return Err(self.error_current("rest element must be the last one"));
                }
                break;
            }

            elements.push(Some(ArrayElement::Expression(
                self.parse_assignment_target_with_default()?,
            )));
            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = self.expect_span(TokenTag::RightBracket)?;
        Ok(Expression::Array(ArrayExpression {
            elements,
            span: Span::between(start, end),
        }))
    }

    fn parse_object_assignment_pattern_expression(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect_span(TokenTag::LeftBrace)?;
        let mut properties = Vec::new();

        while !self.at(TokenTag::RightBrace) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Ellipsis) {
                let spread_start = self.previous().span;
                let argument = self.parse_assignment_target_expression()?;
                let span = Span::between(spread_start, argument.span());
                properties.push(ObjectProperty::Spread { argument, span });
                if self.eat(TokenTag::Comma) {
                    return Err(self.error_current("rest property must be the last one"));
                }
                break;
            }

            let key = self.parse_property_key()?;
            let (mut value, shorthand) = if self.eat(TokenTag::Colon) {
                (self.parse_assignment_target_with_default()?, false)
            } else if let PropertyKey::Identifier(identifier) = key.clone() {
                self.ensure_identifier_name_allowed(&identifier.name, identifier.span)?;
                (Expression::Identifier(identifier), true)
            } else {
                return Err(self.error_current("object pattern property requires ':'"));
            };

            if self.eat(TokenTag::Assign) {
                let right = self.parse_assignment_expression(true)?;
                let span = Span::between(value.span(), right.span());
                value = Expression::Assignment(Box::new(AssignmentExpression {
                    operator: AssignmentOperator::Assign,
                    left: value,
                    right,
                    span,
                }));
            }

            let span = Span::between(key.span(), value.span());
            properties.push(ObjectProperty::Property {
                key,
                value,
                shorthand,
                kind: ObjectPropertyKind::Init,
                span,
            });

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        let end = self.expect_span(TokenTag::RightBrace)?;
        Ok(Expression::Object(ObjectExpression {
            properties,
            span: Span::between(start, end),
        }))
    }

    fn parse_property_key(&mut self) -> Result<PropertyKey, ParseError> {
        self.parse_property_key_with_private(false)
    }

    fn parse_property_key_with_private(
        &mut self,
        allow_private: bool,
    ) -> Result<PropertyKey, ParseError> {
        if self.eat(TokenTag::LeftBracket) {
            let start = self.previous().span;
            let expression = self.parse_expression(true)?;
            let end = self.expect_span(TokenTag::RightBracket)?;
            return Ok(PropertyKey::Computed {
                expression: Box::new(expression),
                span: Span::between(start, end),
            });
        }

        let token = self.advance();
        match token.kind {
            TokenKind::Identifier(name) => Ok(PropertyKey::Identifier(Identifier {
                name,
                span: token.span,
            })),
            TokenKind::PrivateName(name) if allow_private => {
                Ok(PropertyKey::PrivateName(Identifier {
                    name,
                    span: token.span,
                }))
            }
            TokenKind::String(value) => Ok(PropertyKey::String(StringLiteral {
                value,
                span: token.span,
            })),
            TokenKind::Number(raw) => Ok(PropertyKey::Number(NumberLiteral {
                raw,
                span: token.span,
            })),
            kind => {
                if let Some(name) = kind.keyword_text() {
                    Ok(PropertyKey::Identifier(Identifier {
                        name: name.to_string(),
                        span: token.span,
                    }))
                } else {
                    Err(ParseError::new("invalid property name", token.span))
                }
            }
        }
    }

    fn parse_object_method_property(
        &mut self,
    ) -> Result<(PropertyKey, Expression, bool, ObjectPropertyKind), ParseError> {
        let start = self.current().span;

        if self.at_object_accessor_start() {
            let accessor_name = self.advance();
            let kind = match accessor_name.kind.identifier() {
                Some("get") => ObjectPropertyKind::Getter,
                Some("set") => ObjectPropertyKind::Setter,
                _ => unreachable!(),
            };
            let key = self.parse_property_key()?;
            let function = self.parse_method_function(start, false, false)?;
            return Ok((key, Expression::Function(Box::new(function)), false, kind));
        }

        let (is_async, is_generator) = if self.at_async_generator_method_prefix(false) {
            self.advance();
            self.expect(TokenTag::Mul)?;
            (true, true)
        } else if self.at_async_method_prefix(false) {
            self.advance();
            (true, false)
        } else {
            (false, self.eat(TokenTag::Mul))
        };
        let key = self.parse_property_key()?;
        let function = self.parse_method_function(start, is_async, is_generator)?;
        Ok((
            key,
            Expression::Function(Box::new(function)),
            false,
            ObjectPropertyKind::Method,
        ))
    }

    fn at_arrow_start(&self) -> bool {
        (self.at_binding_identifier()
            && self.peek_tag(1) == Some(TokenTag::Arrow)
            && self.peek(1).is_some_and(|token| !token.leading_line_break))
            || (self.at(TokenTag::LeftParen) && self.lookahead_paren_arrow(self.index))
    }

    fn at_async_arrow_start(&self) -> bool {
        self.current_is_identifier_named("async")
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && ((self.peek_tag(1) == Some(TokenTag::Identifier)
                && self.peek_tag(2) == Some(TokenTag::Arrow)
                && self.peek(2).is_some_and(|token| !token.leading_line_break))
                || (self.peek_tag(1) == Some(TokenTag::LeftParen)
                    && self.lookahead_paren_arrow(self.index + 1)))
    }

    fn lookahead_paren_arrow(&self, start_index: usize) -> bool {
        if self.tokens.get(start_index).map(Token::tag) != Some(TokenTag::LeftParen) {
            return false;
        }

        let mut depth = 0usize;
        for (index, token) in self.tokens.iter().enumerate().skip(start_index) {
            match token.tag() {
                TokenTag::LeftParen => depth += 1,
                TokenTag::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return self.tokens.get(index + 1).is_some_and(|token| {
                            token.tag() == TokenTag::Arrow && !token.leading_line_break
                        });
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn at_object_method_start(&self) -> bool {
        self.at_object_accessor_start()
            || self.at_async_generator_method_prefix(false)
            || self.at_async_method_prefix(false)
            || self.at(TokenTag::Mul)
            || self.property_key_followed_by(self.index, false, TokenTag::LeftParen)
    }

    fn at_object_accessor_start(&self) -> bool {
        (self.current_is_identifier_named("get") || self.current_is_identifier_named("set"))
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.property_key_followed_by(self.index + 1, false, TokenTag::LeftParen)
    }

    fn at_class_accessor_start(&self) -> bool {
        (self.current_is_identifier_named("get") || self.current_is_identifier_named("set"))
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.property_key_followed_by(self.index + 1, true, TokenTag::LeftParen)
    }

    fn at_auto_accessor_field_start(&self) -> bool {
        self.current_is_identifier_named("accessor")
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self
                .lookahead_property_key_end(self.index + 1, true)
                .is_some()
    }

    fn at_async_method_prefix(&self, allow_private: bool) -> bool {
        self.current_is_identifier_named("async")
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.property_key_followed_by(self.index + 1, allow_private, TokenTag::LeftParen)
    }

    fn at_async_generator_method_prefix(&self, allow_private: bool) -> bool {
        self.current_is_identifier_named("async")
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.peek_tag(1) == Some(TokenTag::Mul)
            && self.property_key_followed_by(self.index + 2, allow_private, TokenTag::LeftParen)
    }

    fn at_class_static_modifier(&self) -> bool {
        self.at(TokenTag::Static)
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && !matches!(
                self.peek_tag(1),
                Some(TokenTag::LeftParen | TokenTag::Assign | TokenTag::Semicolon)
            )
    }

    fn at_static_block_start(&self) -> bool {
        self.at(TokenTag::Static)
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.peek_tag(1) == Some(TokenTag::LeftBrace)
    }

    fn property_key_followed_by(
        &self,
        start_index: usize,
        allow_private: bool,
        expected: TokenTag,
    ) -> bool {
        self.lookahead_property_key_end(start_index, allow_private)
            .and_then(|index| self.tokens.get(index))
            .map(Token::tag)
            == Some(expected)
    }

    fn lookahead_property_key_end(&self, start_index: usize, allow_private: bool) -> Option<usize> {
        let token = self.tokens.get(start_index)?;
        match token.tag() {
            TokenTag::LeftBracket => {
                let mut depth = 0usize;
                for (index, token) in self.tokens.iter().enumerate().skip(start_index) {
                    match token.tag() {
                        TokenTag::LeftBracket => depth += 1,
                        TokenTag::RightBracket => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(index + 1);
                            }
                        }
                        _ => {}
                    }
                }
                None
            }
            TokenTag::Identifier | TokenTag::String | TokenTag::Number => Some(start_index + 1),
            TokenTag::PrivateName if allow_private => Some(start_index + 1),
            _ if token.kind.keyword_text().is_some() => Some(start_index + 1),
            _ => None,
        }
    }

    fn is_constructor_key(&self, key: &PropertyKey) -> bool {
        matches!(key, PropertyKey::Identifier(identifier) if identifier.name == "constructor")
    }

    fn consume_class_element_terminator(&mut self) -> Result<(), ParseError> {
        if self.eat(TokenTag::Semicolon) {
            return Ok(());
        }
        if self.at(TokenTag::RightBrace) || self.current().leading_line_break {
            return Ok(());
        }
        Err(self.error_current("expected class element terminator"))
    }

    fn parse_chain_expression(
        &mut self,
        mut expression: Expression,
    ) -> Result<Expression, ParseError> {
        loop {
            let start = expression.span();
            expression = match self.current_tag() {
                TokenTag::Dot => self.parse_named_member_suffix(expression, false)?,
                TokenTag::LeftBracket => self.parse_computed_member_suffix(expression, false)?,
                TokenTag::OptionalChain => self.parse_optional_chain_suffix(expression)?,
                TokenTag::Template => self.parse_tagged_template(expression)?,
                TokenTag::LeftParen => {
                    let arguments = self.parse_arguments()?;
                    let end = arguments.last().map_or(start, CallArgument::span);
                    Expression::Call(Box::new(CallExpression {
                        callee: expression,
                        arguments,
                        optional: false,
                        span: Span::between(start, end),
                    }))
                }
                _ => break,
            };
        }
        Ok(expression)
    }

    fn parse_tagged_template(&mut self, tag: Expression) -> Result<Expression, ParseError> {
        if Self::expression_contains_optional_chain(&tag) {
            return Err(ParseError::new(
                "optional chaining cannot be used in tagged templates",
                tag.span(),
            ));
        }
        let quasi = self.parse_template_literal()?;
        let span = Span::between(tag.span(), quasi.span);
        Ok(Expression::TaggedTemplate(Box::new(
            TaggedTemplateExpression { tag, quasi, span },
        )))
    }

    fn parse_template_literal(&mut self) -> Result<TemplateLiteral, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::Template { value, .. } => Ok(TemplateLiteral {
                value,
                span: token.span,
            }),
            _ => Err(ParseError::new("template literal expected", token.span)),
        }
    }

    fn parse_member_suffix_chain(
        &mut self,
        mut expression: Expression,
    ) -> Result<Expression, ParseError> {
        loop {
            expression = match self.current_tag() {
                TokenTag::Dot => self.parse_named_member_suffix(expression, false)?,
                TokenTag::LeftBracket => self.parse_computed_member_suffix(expression, false)?,
                _ => break,
            };
        }
        Ok(expression)
    }

    fn parse_optional_chain_suffix(
        &mut self,
        expression: Expression,
    ) -> Result<Expression, ParseError> {
        self.expect(TokenTag::OptionalChain)?;
        let start = expression.span();
        if self.at(TokenTag::LeftParen) {
            let arguments = self.parse_arguments()?;
            let end = arguments.last().map_or(start, CallArgument::span);
            return Ok(Expression::Call(Box::new(CallExpression {
                callee: expression,
                arguments,
                optional: true,
                span: Span::between(start, end),
            })));
        }

        if self.at(TokenTag::LeftBracket) {
            return self.parse_computed_member_suffix(expression, true);
        }

        self.parse_named_member_from(expression, true)
    }

    fn parse_named_member_suffix(
        &mut self,
        expression: Expression,
        optional: bool,
    ) -> Result<Expression, ParseError> {
        self.expect(TokenTag::Dot)?;
        self.parse_named_member_from(expression, optional)
    }

    fn parse_named_member_from(
        &mut self,
        expression: Expression,
        optional: bool,
    ) -> Result<Expression, ParseError> {
        let token = self.advance();
        let property = match token.kind {
            TokenKind::Identifier(name) => MemberProperty::Identifier(Identifier {
                name,
                span: token.span,
            }),
            TokenKind::PrivateName(name) => MemberProperty::PrivateName(Identifier {
                name,
                span: token.span,
            }),
            kind => {
                if let Some(name) = kind.keyword_text() {
                    MemberProperty::Identifier(Identifier {
                        name: name.to_string(),
                        span: token.span,
                    })
                } else {
                    return Err(ParseError::new("expecting property name", token.span));
                }
            }
        };
        let end = property.span();
        let start = expression.span();
        Ok(Expression::Member(Box::new(MemberExpression {
            object: expression,
            property,
            optional,
            span: Span::between(start, end),
        })))
    }

    fn parse_computed_member_suffix(
        &mut self,
        expression: Expression,
        optional: bool,
    ) -> Result<Expression, ParseError> {
        let start = expression.span();
        self.expect(TokenTag::LeftBracket)?;
        let property = self.parse_expression(true)?;
        let end = self.expect_span(TokenTag::RightBracket)?;
        Ok(Expression::Member(Box::new(MemberExpression {
            object: expression,
            property: MemberProperty::Computed {
                expression: Box::new(property),
                span: Span::between(start, end),
            },
            optional,
            span: Span::between(start, end),
        })))
    }

    fn parse_arguments(&mut self) -> Result<Vec<CallArgument>, ParseError> {
        self.expect(TokenTag::LeftParen)?;
        let mut arguments = Vec::new();

        while !self.at(TokenTag::RightParen) && !self.at(TokenTag::Eof) {
            if self.eat(TokenTag::Ellipsis) {
                let spread_start = self.previous().span;
                let argument = self.parse_assignment_expression(true)?;
                let span = Span::between(spread_start, argument.span());
                arguments.push(CallArgument::Spread { argument, span });
            } else {
                arguments.push(CallArgument::Expression(
                    self.parse_assignment_expression(true)?,
                ));
            }

            if !self.eat(TokenTag::Comma) {
                break;
            }
        }

        self.expect(TokenTag::RightParen)?;
        Ok(arguments)
    }

    fn parse_binding_identifier(&mut self) -> Result<Identifier, ParseError> {
        let token = self.current().clone();
        let name = self
            .current_binding_identifier_name()
            .ok_or_else(|| ParseError::new("identifier expected", token.span))?;
        if token.escaped
            && self.escaped_identifier_is_reserved_here(
                &name,
                self.is_strict,
                self.await_context,
                self.yield_context,
            )
        {
            return Err(ParseError::new(
                format!("escaped reserved word '{name}' is not allowed here"),
                token.span,
            ));
        }
        let span = self.advance_span();
        Ok(Identifier { name, span })
    }

    fn parse_decorator_list(&mut self) -> Result<Vec<Expression>, ParseError> {
        let mut decorators = Vec::new();
        while self.eat(TokenTag::At) {
            decorators.push(self.parse_left_hand_side_expression()?);
        }
        Ok(decorators)
    }

    fn current_assignment_operator(&self) -> Option<AssignmentOperator> {
        Some(match self.current_tag() {
            TokenTag::Assign => AssignmentOperator::Assign,
            TokenTag::AddAssign => AssignmentOperator::AddAssign,
            TokenTag::SubAssign => AssignmentOperator::SubAssign,
            TokenTag::MulAssign => AssignmentOperator::MulAssign,
            TokenTag::DivAssign => AssignmentOperator::DivAssign,
            TokenTag::ModAssign => AssignmentOperator::ModAssign,
            TokenTag::PowAssign => AssignmentOperator::PowAssign,
            TokenTag::ShlAssign => AssignmentOperator::ShlAssign,
            TokenTag::SarAssign => AssignmentOperator::SarAssign,
            TokenTag::ShrAssign => AssignmentOperator::ShrAssign,
            TokenTag::AndAssign => AssignmentOperator::AndAssign,
            TokenTag::XorAssign => AssignmentOperator::XorAssign,
            TokenTag::OrAssign => AssignmentOperator::OrAssign,
            TokenTag::LogicalAndAssign => AssignmentOperator::LogicalAndAssign,
            TokenTag::LogicalOrAssign => AssignmentOperator::LogicalOrAssign,
            TokenTag::NullishAssign => AssignmentOperator::NullishAssign,
            _ => return None,
        })
    }

    fn at_assignment_pattern_start(&self) -> bool {
        matches!(
            self.current_tag(),
            TokenTag::LeftBrace | TokenTag::LeftBracket
        ) && self.lookahead_assignment_pattern_end() == Some(TokenTag::Assign)
    }

    fn assignment_target_uses_left_hand_side_expression(&self) -> bool {
        matches!(
            self.lookahead_assignment_pattern_end(),
            Some(
                TokenTag::Dot
                    | TokenTag::LeftBracket
                    | TokenTag::LeftParen
                    | TokenTag::OptionalChain
                    | TokenTag::Template
            )
        )
    }

    fn lookahead_assignment_pattern_end(&self) -> Option<TokenTag> {
        let mut stack = Vec::new();

        for (index, token) in self.tokens.iter().enumerate().skip(self.index) {
            match token.tag() {
                TokenTag::LeftParen => stack.push(TokenTag::RightParen),
                TokenTag::LeftBracket => stack.push(TokenTag::RightBracket),
                TokenTag::LeftBrace => stack.push(TokenTag::RightBrace),
                TokenTag::RightParen | TokenTag::RightBracket | TokenTag::RightBrace => {
                    if stack.pop() != Some(token.tag()) {
                        return None;
                    }
                    if stack.is_empty() {
                        return self.tokens.get(index + 1).map(Token::tag);
                    }
                }
                _ => {}
            }
        }

        None
    }

    fn expression_is_for_in_of_left(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Identifier(_)
            | Expression::Array(_)
            | Expression::Object(_)
            | Expression::Call(_) => true,
            Expression::Member(member) => !member.optional,
            _ => false,
        }
    }

    fn current_infix_operator(&self, allow_in: bool) -> Option<(InfixOperator, u8, u8)> {
        Some(match self.current_tag() {
            TokenTag::LogicalOr => (InfixOperator::Logical(LogicalOperator::Or), 1, 2),
            TokenTag::NullishCoalescing => (
                InfixOperator::Logical(LogicalOperator::NullishCoalescing),
                2,
                3,
            ),
            TokenTag::LogicalAnd => (InfixOperator::Logical(LogicalOperator::And), 3, 4),
            TokenTag::BitOr => (InfixOperator::Binary(BinaryOperator::BitwiseOr), 4, 5),
            TokenTag::BitXor => (InfixOperator::Binary(BinaryOperator::BitwiseXor), 5, 6),
            TokenTag::BitAnd => (InfixOperator::Binary(BinaryOperator::BitwiseAnd), 6, 7),
            TokenTag::Eq => (InfixOperator::Binary(BinaryOperator::Equality), 7, 8),
            TokenTag::StrictEq => (InfixOperator::Binary(BinaryOperator::StrictEquality), 7, 8),
            TokenTag::Ne => (InfixOperator::Binary(BinaryOperator::Inequality), 7, 8),
            TokenTag::StrictNe => (
                InfixOperator::Binary(BinaryOperator::StrictInequality),
                7,
                8,
            ),
            TokenTag::Lt => (InfixOperator::Binary(BinaryOperator::LessThan), 8, 9),
            TokenTag::Lte => (InfixOperator::Binary(BinaryOperator::LessThanOrEqual), 8, 9),
            TokenTag::Gt => (InfixOperator::Binary(BinaryOperator::GreaterThan), 8, 9),
            TokenTag::Gte => (
                InfixOperator::Binary(BinaryOperator::GreaterThanOrEqual),
                8,
                9,
            ),
            TokenTag::In if allow_in => (InfixOperator::Binary(BinaryOperator::In), 8, 9),
            TokenTag::Instanceof => (InfixOperator::Binary(BinaryOperator::Instanceof), 8, 9),
            TokenTag::Shl => (InfixOperator::Binary(BinaryOperator::LeftShift), 9, 10),
            TokenTag::Sar => (
                InfixOperator::Binary(BinaryOperator::SignedRightShift),
                9,
                10,
            ),
            TokenTag::Shr => (
                InfixOperator::Binary(BinaryOperator::UnsignedRightShift),
                9,
                10,
            ),
            TokenTag::Add => (InfixOperator::Binary(BinaryOperator::Add), 10, 11),
            TokenTag::Sub => (InfixOperator::Binary(BinaryOperator::Subtract), 10, 11),
            TokenTag::Mul => (InfixOperator::Binary(BinaryOperator::Multiply), 11, 12),
            TokenTag::Div => (InfixOperator::Binary(BinaryOperator::Divide), 11, 12),
            TokenTag::Mod => (InfixOperator::Binary(BinaryOperator::Modulo), 11, 12),
            TokenTag::Pow => (InfixOperator::Binary(BinaryOperator::Exponentiate), 12, 12),
            _ => return None,
        })
    }

    fn consume_semicolon(&mut self) -> Result<(), ParseError> {
        if self.eat(TokenTag::Semicolon) {
            return Ok(());
        }
        if self.statement_is_terminated_here() {
            return Ok(());
        }
        Err(self.error_current("expected ';'"))
    }

    fn statement_is_terminated_here(&self) -> bool {
        self.at(TokenTag::Semicolon)
            || self.at(TokenTag::Eof)
            || self.at(TokenTag::RightBrace)
            || self.current().leading_line_break
    }

    fn at_async_function(&self) -> bool {
        self.current_is_identifier_named("async")
            && self.peek_tag(1) == Some(TokenTag::Function)
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
    }

    fn current_is_contextual_of(&self) -> bool {
        self.current_is_identifier_named("of")
    }

    fn at_let_declaration_start(&self) -> bool {
        self.declaration_binding_start_at(self.index + 1)
    }

    fn at_binding_identifier(&self) -> bool {
        self.current_binding_identifier_name().is_some()
    }

    fn at_function_expression_name(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Identifier(_) | TokenKind::Yield | TokenKind::Await | TokenKind::Let
        ) || self
            .relaxed_keyword_identifier_name(&self.current().kind)
            .is_some()
    }

    fn current_binding_identifier_name(&self) -> Option<String> {
        match &self.current().kind {
            TokenKind::Identifier(name) => Some(name.clone()),
            TokenKind::Yield if !self.yield_context => Some("yield".to_string()),
            TokenKind::Await if !self.await_context => Some("await".to_string()),
            TokenKind::Let if self.allow_let_binding_identifier => Some("let".to_string()),
            kind => self
                .relaxed_keyword_identifier_name(kind)
                .map(ToString::to_string),
        }
    }

    fn at_using_declaration_start(&self) -> bool {
        self.current_is_identifier_named("using")
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.declaration_identifier_start_at(self.index + 1)
    }

    fn at_await_using_declaration_start(&self) -> bool {
        self.await_context
            && self.at(TokenTag::Await)
            && self.peek(1).is_some_and(|token| !token.leading_line_break)
            && self.peek(1).and_then(|token| token.kind.identifier()) == Some("using")
            && self.peek(2).is_some_and(|token| !token.leading_line_break)
            && self.declaration_identifier_start_at(self.index + 2)
    }

    fn at_for_head_declaration_start(&self) -> bool {
        match self.current_tag() {
            TokenTag::Var | TokenTag::Const => true,
            TokenTag::Let => self.at_let_declaration_start(),
            _ if self.at_await_using_declaration_start() => true,
            _ if self.at_using_declaration_start() => true,
            _ => false,
        }
    }

    fn declaration_binding_start_at(&self, index: usize) -> bool {
        let Some(token) = self.tokens.get(index) else {
            return false;
        };

        matches!(token.tag(), TokenTag::LeftBracket | TokenTag::LeftBrace)
            || matches!(
                token.kind,
                TokenKind::Identifier(_) | TokenKind::Yield | TokenKind::Await | TokenKind::Let
            )
            || self.relaxed_keyword_identifier_name(&token.kind).is_some()
    }

    fn declaration_identifier_start_at(&self, index: usize) -> bool {
        let Some(token) = self.tokens.get(index) else {
            return false;
        };

        matches!(
            token.kind,
            TokenKind::Identifier(_) | TokenKind::Yield | TokenKind::Await | TokenKind::Let
        ) || self.relaxed_keyword_identifier_name(&token.kind).is_some()
    }

    fn relaxed_keyword_identifier_name(&self, kind: &TokenKind) -> Option<&'static str> {
        match kind {
            TokenKind::Public => Some("public"),
            TokenKind::Private => Some("private"),
            TokenKind::Protected => Some("protected"),
            TokenKind::Static => Some("static"),
            TokenKind::Package => Some("package"),
            TokenKind::Interface => Some("interface"),
            TokenKind::Implements => Some("implements"),
            TokenKind::Enum => Some("enum"),
            _ => None,
        }
    }

    fn ensure_identifier_name_allowed(&self, name: &str, span: Span) -> Result<(), ParseError> {
        if (name == "yield" && self.yield_context) || (name == "await" && self.await_context) {
            return Err(ParseError::new(
                format!("identifier '{name}' is reserved here"),
                span,
            ));
        }
        Ok(())
    }

    fn with_let_identifier_allowed<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let previous = self.allow_let_identifier;
        self.allow_let_identifier = true;
        let result = f(self);
        self.allow_let_identifier = previous;
        result
    }

    fn with_let_binding_identifier_allowed<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let previous = self.allow_let_binding_identifier;
        self.allow_let_binding_identifier = true;
        let result = f(self);
        self.allow_let_binding_identifier = previous;
        result
    }

    fn with_field_initializer_context<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let previous_await_context = self.await_context;
        if self.function_depth > 0 {
            self.await_context = false;
        }
        let result = f(self);
        self.await_context = previous_await_context;
        result
    }

    fn yield_has_argument(&self) -> bool {
        !self.current().leading_line_break
            && !matches!(
                self.current_tag(),
                TokenTag::Semicolon
                    | TokenTag::RightParen
                    | TokenTag::RightBracket
                    | TokenTag::RightBrace
                    | TokenTag::Comma
                    | TokenTag::Colon
                    | TokenTag::Eof
            )
    }

    fn with_function_context<T>(
        &mut self,
        is_async: bool,
        is_generator: bool,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let previous_function_depth = self.function_depth;
        let previous_await_context = self.await_context;
        let previous_yield_context = self.yield_context;
        self.function_depth += 1;
        self.await_context = is_async;
        self.yield_context = is_generator;
        let result = f(self);
        self.function_depth = previous_function_depth;
        self.await_context = previous_await_context;
        self.yield_context = previous_yield_context;
        result
    }

    fn validate_program(&self, program: &Program) -> Result<(), ParseError> {
        let private_names = HashSet::new();
        let context = ValidationContext {
            strict: self.is_strict || self.statement_list_has_use_strict(&program.body),
            await_reserved: self.await_context,
            yield_reserved: false,
            in_class_static_block: false,
            module_await_reserved: self.await_context,
            return_allowed: false,
            new_target_allowed: false,
            super_property_allowed: false,
            super_call_allowed: false,
        };
        self.validate_statement_list_control(&program.body, context, &[], false, false)?;
        self.validate_statement_list(
            &program.body,
            context,
            &private_names,
            if self.is_module {
                StatementListKind::Module
            } else {
                StatementListKind::Script
            },
        )?;
        if self.is_module {
            self.validate_module_export_early_errors(program)?;
        }
        Ok(())
    }

    fn validate_statement_list(
        &self,
        statements: &[Statement],
        context: ValidationContext,
        private_names: &HashSet<String>,
        kind: StatementListKind,
    ) -> Result<(), ParseError> {
        self.validate_statement_list_early_errors(statements, context, kind)?;
        for statement in statements {
            self.validate_statement(statement, context, private_names, kind)?;
        }
        Ok(())
    }

    fn validate_statement(
        &self,
        statement: &Statement,
        context: ValidationContext,
        private_names: &HashSet<String>,
        list_kind: StatementListKind,
    ) -> Result<(), ParseError> {
        match statement {
            Statement::Directive(_) | Statement::Empty(_) | Statement::Debugger(_) => Ok(()),
            Statement::Block(block) => self.validate_statement_list(
                &block.body,
                context,
                private_names,
                StatementListKind::Block,
            ),
            Statement::Labeled(statement) => {
                self.validate_identifier_reference(&statement.label, context)?;
                if !Self::is_labelled_function(statement.body.as_ref()) {
                    self.validate_statement_position(statement.body.as_ref(), !context.strict)?;
                }
                self.validate_statement(statement.body.as_ref(), context, private_names, list_kind)
            }
            Statement::ImportDeclaration(declaration) => {
                if !list_kind.allow_module_declarations() {
                    return Err(ParseError::new(
                        "import declarations are only allowed at the top level of module code",
                        declaration.span,
                    ));
                }
                self.validate_import_declaration(declaration, context)?;
                if let Some(attributes) = &declaration.attributes {
                    self.validate_expression(attributes, context, private_names)?;
                }
                Ok(())
            }
            Statement::ExportDeclaration(declaration) => {
                if !list_kind.allow_module_declarations() {
                    return Err(ParseError::new(
                        "export declarations are only allowed at the top level of module code",
                        declaration.span(),
                    ));
                }
                self.validate_export_declaration(declaration, context, private_names)
            }
            Statement::VariableDeclaration(declaration) => {
                if matches!(
                    declaration.kind,
                    VariableKind::Using | VariableKind::AwaitUsing
                ) && ((!self.is_module && list_kind == StatementListKind::Script)
                    || list_kind == StatementListKind::SwitchCase)
                {
                    return Err(ParseError::new(
                        "using declarations are not allowed here",
                        declaration.span,
                    ));
                }
                self.validate_variable_declaration(declaration, context, private_names)
            }
            Statement::FunctionDeclaration(function) => {
                self.validate_function(function, context, private_names)
            }
            Statement::ClassDeclaration(class) => {
                self.validate_class(class, context, private_names)
            }
            Statement::If(statement) => {
                self.validate_expression(&statement.test, context, private_names)?;
                self.validate_statement_position(statement.consequent.as_ref(), !context.strict)?;
                self.validate_statement(
                    statement.consequent.as_ref(),
                    context,
                    private_names,
                    list_kind,
                )?;
                if let Some(alternate) = &statement.alternate {
                    self.validate_statement_position(alternate.as_ref(), !context.strict)?;
                    self.validate_statement(alternate.as_ref(), context, private_names, list_kind)?;
                }
                Ok(())
            }
            Statement::While(statement) => {
                self.validate_expression(&statement.test, context, private_names)?;
                self.validate_statement_position(statement.body.as_ref(), false)?;
                self.validate_statement(statement.body.as_ref(), context, private_names, list_kind)
            }
            Statement::DoWhile(statement) => {
                self.validate_statement_position(statement.body.as_ref(), false)?;
                self.validate_statement(
                    statement.body.as_ref(),
                    context,
                    private_names,
                    list_kind,
                )?;
                self.validate_expression(&statement.test, context, private_names)
            }
            Statement::For(statement) => {
                self.validate_for_statement(statement, context, private_names, list_kind)
            }
            Statement::Switch(statement) => {
                self.validate_switch_case_early_errors(&statement.cases, context)?;
                self.validate_expression(&statement.discriminant, context, private_names)?;
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        self.validate_expression(test, context, private_names)?;
                    }
                    self.validate_statement_list(
                        &case.consequent,
                        context,
                        private_names,
                        StatementListKind::SwitchCase,
                    )?;
                }
                Ok(())
            }
            Statement::Return(statement) => {
                if !context.return_allowed {
                    return Err(ParseError::new(
                        "return statements are only allowed inside function bodies",
                        statement.span,
                    ));
                }
                if let Some(argument) = &statement.argument {
                    self.validate_expression(argument, context, private_names)?;
                }
                Ok(())
            }
            Statement::Break(_) | Statement::Continue(_) => Ok(()),
            Statement::Throw(statement) => {
                self.validate_expression(&statement.argument, context, private_names)
            }
            Statement::Try(statement) => {
                self.validate_statement_list(
                    &statement.block.body,
                    context,
                    private_names,
                    StatementListKind::Block,
                )?;
                if let Some(handler) = &statement.handler {
                    if let Some(param) = &handler.param {
                        if let Some(duplicate) =
                            self.first_duplicate_bound_name(std::slice::from_ref(param))
                        {
                            return Err(ParseError::new(
                                "duplicate binding name in catch parameter",
                                duplicate.span,
                            ));
                        }
                        self.validate_pattern(param, context, private_names)?;
                        self.validate_catch_parameter_conflicts(param, &handler.body.body)?;
                    }
                    self.validate_statement_list(
                        &handler.body.body,
                        context,
                        private_names,
                        StatementListKind::Block,
                    )?;
                }
                if let Some(finalizer) = &statement.finalizer {
                    self.validate_statement_list(
                        &finalizer.body,
                        context,
                        private_names,
                        StatementListKind::Block,
                    )?;
                }
                Ok(())
            }
            Statement::With(statement) => {
                if context.strict {
                    return Err(ParseError::new(
                        "with statements are not allowed in strict mode",
                        statement.span,
                    ));
                }
                self.validate_expression(&statement.object, context, private_names)?;
                self.validate_statement_position(statement.body.as_ref(), false)?;
                self.validate_statement(statement.body.as_ref(), context, private_names, list_kind)
            }
            Statement::Expression(statement) => {
                if self.expression_starts_with_linebreak_let_bracket(&statement.expression) {
                    return Err(ParseError::new(
                        "expression statements cannot start with 'let ['",
                        statement.expression.span(),
                    ));
                }
                self.validate_expression(&statement.expression, context, private_names)
            }
        }
    }

    fn validate_import_declaration(
        &self,
        declaration: &ImportDeclaration,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        if let Some(attributes) = &declaration.attributes {
            self.validate_module_attributes(attributes)?;
        }
        let Some(clause) = &declaration.clause else {
            return Ok(());
        };
        if let Some(duplicate) = self.first_duplicate_import_binding(clause) {
            return Err(ParseError::new("duplicate import binding", duplicate.span));
        }
        self.validate_import_clause(clause, context)
    }

    fn validate_import_clause(
        &self,
        clause: &ImportClause,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        match clause {
            ImportClause::Default(identifier) => {
                self.validate_binding_identifier(identifier, context)
            }
            ImportClause::Namespace { default, namespace } => {
                if let Some(identifier) = default {
                    self.validate_binding_identifier(identifier, context)?;
                }
                self.validate_binding_identifier(namespace, context)
            }
            ImportClause::Named {
                default,
                specifiers,
            } => {
                if let Some(identifier) = default {
                    self.validate_binding_identifier(identifier, context)?;
                }
                for specifier in specifiers {
                    self.validate_binding_identifier(&specifier.local, context)?;
                }
                Ok(())
            }
        }
    }

    fn validate_statement_list_early_errors(
        &self,
        statements: &[Statement],
        context: ValidationContext,
        kind: StatementListKind,
    ) -> Result<(), ParseError> {
        if let Some(duplicate) =
            self.first_duplicate_lexically_declared_name(statements, kind, context.strict)
        {
            return Err(ParseError::new(
                "duplicate lexical declaration",
                duplicate.span,
            ));
        }
        if let Some(conflict) = self.first_lexical_var_conflict(statements, kind) {
            return Err(ParseError::new(
                "lexical declaration conflicts with var declaration",
                conflict.span,
            ));
        }
        if kind == StatementListKind::Module
            && let Some(duplicate) = self.first_duplicate_exported_name(statements)
        {
            return Err(ParseError::new("duplicate export name", duplicate.span));
        }
        if let Some(duplicate) = self.first_duplicate_label_name(statements) {
            return Err(ParseError::new("duplicate label", duplicate.span));
        }
        Ok(())
    }

    fn validate_statement_list_control(
        &self,
        statements: &[Statement],
        context: ValidationContext,
        labels: &[LabelContext],
        in_iteration: bool,
        in_switch: bool,
    ) -> Result<(), ParseError> {
        for statement in statements {
            self.validate_statement_control(statement, context, labels, in_iteration, in_switch)?;
        }
        Ok(())
    }

    fn validate_statement_control(
        &self,
        statement: &Statement,
        context: ValidationContext,
        labels: &[LabelContext],
        in_iteration: bool,
        in_switch: bool,
    ) -> Result<(), ParseError> {
        match statement {
            Statement::Directive(_)
            | Statement::Empty(_)
            | Statement::Debugger(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportDeclaration(_)
            | Statement::VariableDeclaration(_)
            | Statement::Expression(_)
            | Statement::Throw(_)
            | Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_) => Ok(()),
            Statement::Block(block) => self.validate_statement_list_control(
                &block.body,
                context,
                labels,
                in_iteration,
                in_switch,
            ),
            Statement::Labeled(statement) => {
                if labels
                    .iter()
                    .any(|label| label.name == statement.label.name)
                {
                    return Err(ParseError::new("duplicate label", statement.label.span));
                }
                let mut nested_labels = labels.to_vec();
                nested_labels.push(LabelContext {
                    name: statement.label.name.clone(),
                    continue_allowed: matches!(
                        statement.body.as_ref(),
                        Statement::While(_) | Statement::DoWhile(_) | Statement::For(_)
                    ),
                });
                self.validate_statement_control(
                    statement.body.as_ref(),
                    context,
                    &nested_labels,
                    in_iteration,
                    in_switch,
                )
            }
            Statement::If(statement) => {
                self.validate_statement_control(
                    statement.consequent.as_ref(),
                    context,
                    labels,
                    in_iteration,
                    in_switch,
                )?;
                if let Some(alternate) = &statement.alternate {
                    self.validate_statement_control(
                        alternate.as_ref(),
                        context,
                        labels,
                        in_iteration,
                        in_switch,
                    )?;
                }
                Ok(())
            }
            Statement::While(statement) => self.validate_statement_control(
                statement.body.as_ref(),
                context,
                labels,
                true,
                in_switch,
            ),
            Statement::DoWhile(statement) => self.validate_statement_control(
                statement.body.as_ref(),
                context,
                labels,
                true,
                in_switch,
            ),
            Statement::For(statement) => match statement {
                ForStatement::Classic(statement) => self.validate_statement_control(
                    statement.body.as_ref(),
                    context,
                    labels,
                    true,
                    in_switch,
                ),
                ForStatement::In(statement) | ForStatement::Of(statement) => self
                    .validate_statement_control(
                        statement.body.as_ref(),
                        context,
                        labels,
                        true,
                        in_switch,
                    ),
            },
            Statement::Switch(statement) => {
                for case in &statement.cases {
                    self.validate_statement_list_control(
                        &case.consequent,
                        context,
                        labels,
                        in_iteration,
                        true,
                    )?;
                }
                Ok(())
            }
            Statement::Return(_) => Ok(()),
            Statement::Break(statement) => {
                if let Some(label) = &statement.label {
                    if labels.iter().rev().all(|item| item.name != label.name) {
                        return Err(ParseError::new("undefined break label", label.span));
                    }
                } else if !in_iteration && !in_switch {
                    return Err(ParseError::new(
                        "break statements require an enclosing loop, switch, or label",
                        statement.span,
                    ));
                }
                Ok(())
            }
            Statement::Continue(statement) => {
                if let Some(label) = &statement.label {
                    if labels
                        .iter()
                        .rev()
                        .find(|item| item.name == label.name)
                        .is_none_or(|item| !item.continue_allowed)
                    {
                        return Err(ParseError::new(
                            "continue labels must target an iteration statement",
                            label.span,
                        ));
                    }
                } else if !in_iteration {
                    return Err(ParseError::new(
                        "continue statements require an enclosing loop",
                        statement.span,
                    ));
                }
                Ok(())
            }
            Statement::Try(statement) => {
                self.validate_statement_list_control(
                    &statement.block.body,
                    context,
                    labels,
                    in_iteration,
                    in_switch,
                )?;
                if let Some(handler) = &statement.handler {
                    self.validate_statement_list_control(
                        &handler.body.body,
                        context,
                        labels,
                        in_iteration,
                        in_switch,
                    )?;
                }
                if let Some(finalizer) = &statement.finalizer {
                    self.validate_statement_list_control(
                        &finalizer.body,
                        context,
                        labels,
                        in_iteration,
                        in_switch,
                    )?;
                }
                Ok(())
            }
            Statement::With(statement) => self.validate_statement_control(
                statement.body.as_ref(),
                context,
                labels,
                in_iteration,
                in_switch,
            ),
        }
    }

    fn validate_catch_parameter_conflicts(
        &self,
        param: &Pattern,
        body: &[Statement],
    ) -> Result<(), ParseError> {
        let mut parameter_names = Vec::new();
        Self::append_pattern_bound_names(param, &mut parameter_names);
        let parameter_name_set: HashSet<_> = parameter_names
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        let lexical_names =
            self.statement_list_lexically_declared_names(body, StatementListKind::Block);
        if let Some(conflict) = lexical_names
            .iter()
            .find(|name| parameter_name_set.contains(name.name.as_str()))
        {
            return Err(ParseError::new(
                "catch parameter conflicts with lexical declaration",
                conflict.span,
            ));
        }
        Ok(())
    }

    fn validate_switch_case_early_errors(
        &self,
        cases: &[SwitchCase],
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        let mut lexical_entries = Vec::new();
        let mut var_names = Vec::new();
        let mut default_span = None;
        for case in cases {
            if case.test.is_none() {
                if let Some(span) = default_span {
                    return Err(ParseError::new(
                        "switch statements cannot contain more than one default clause",
                        span,
                    ));
                }
                default_span = Some(case.span);
            }
            lexical_entries.extend(self.statement_list_lexically_declared_entries(
                &case.consequent,
                StatementListKind::SwitchCase,
            ));
            var_names.extend(self.statement_list_var_declared_names(
                &case.consequent,
                StatementListKind::SwitchCase,
            ));
        }

        if let Some(duplicate) = self.first_duplicate_lexical_name_in_entries(
            &lexical_entries,
            StatementListKind::SwitchCase,
            context.strict,
        ) {
            return Err(ParseError::new(
                "duplicate lexical declaration",
                duplicate.span,
            ));
        }

        let var_name_set: HashSet<_> = var_names.iter().map(|name| name.name.as_str()).collect();
        if let Some(conflict) = lexical_entries
            .iter()
            .map(|entry| &entry.binding)
            .find(|lexical| var_name_set.contains(lexical.name.as_str()))
        {
            return Err(ParseError::new(
                "lexical declaration conflicts with var declaration",
                conflict.span,
            ));
        }

        Ok(())
    }

    fn first_duplicate_import_binding(&self, clause: &ImportClause) -> Option<NamedSpan> {
        let mut names = Vec::new();
        self.append_import_clause_bound_names(clause, &mut names);
        self.first_duplicate_named_span(&names)
    }

    fn first_duplicate_lexically_declared_name(
        &self,
        statements: &[Statement],
        kind: StatementListKind,
        strict: bool,
    ) -> Option<NamedSpan> {
        let entries = self.statement_list_lexically_declared_entries(statements, kind);
        self.first_duplicate_lexical_name_in_entries(&entries, kind, strict)
    }

    fn first_duplicate_lexical_name_in_entries(
        &self,
        entries: &[LexicalName],
        kind: StatementListKind,
        strict: bool,
    ) -> Option<NamedSpan> {
        #[derive(Clone, Copy)]
        struct DuplicateLexicalState<'a> {
            duplicate_binding: Option<&'a NamedSpan>,
            duplicate_index: usize,
            all_function_declarations: bool,
        }

        let mut declarations = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            declarations
                .entry(entry.binding.name.as_str())
                .and_modify(|state: &mut DuplicateLexicalState<'_>| {
                    state.all_function_declarations &= entry.function_declaration;
                    if state.duplicate_binding.is_none() {
                        state.duplicate_binding = Some(&entry.binding);
                        state.duplicate_index = index;
                    }
                })
                .or_insert(DuplicateLexicalState {
                    duplicate_binding: None,
                    duplicate_index: usize::MAX,
                    all_function_declarations: entry.function_declaration,
                });
        }

        let allow_sloppy_function_duplicates = matches!(
            kind,
            StatementListKind::Block | StatementListKind::SwitchCase
        ) && !strict;

        declarations
            .into_values()
            .filter(|state| state.duplicate_binding.is_some())
            .filter(|state| !(allow_sloppy_function_duplicates && state.all_function_declarations))
            .min_by_key(|state| state.duplicate_index)
            .and_then(|state| state.duplicate_binding.cloned())
    }

    fn first_lexical_var_conflict(
        &self,
        statements: &[Statement],
        kind: StatementListKind,
    ) -> Option<NamedSpan> {
        let lexical_names = self.statement_list_lexically_declared_names(statements, kind);
        let var_names = self.statement_list_var_declared_names(statements, kind);
        let var_name_set: HashSet<_> = var_names.iter().map(|name| name.name.as_str()).collect();
        lexical_names
            .into_iter()
            .find(|lexical| var_name_set.contains(lexical.name.as_str()))
    }

    fn first_duplicate_exported_name(&self, statements: &[Statement]) -> Option<NamedSpan> {
        let mut names = Vec::new();
        for statement in statements {
            self.append_statement_exported_names(statement, &mut names);
        }
        self.first_duplicate_named_span(&names)
    }

    fn first_duplicate_label_name(&self, statements: &[Statement]) -> Option<Identifier> {
        let mut labels = Vec::new();
        for statement in statements {
            if let Some(label) = Self::statement_duplicate_label(statement, &mut labels) {
                return Some(label);
            }
        }
        None
    }

    fn statement_duplicate_label(
        statement: &Statement,
        labels: &mut Vec<String>,
    ) -> Option<Identifier> {
        match statement {
            Statement::Labeled(statement) => {
                if labels.iter().any(|name| name == &statement.label.name) {
                    return Some(statement.label.clone());
                }
                labels.push(statement.label.name.clone());
                let duplicate = Self::statement_duplicate_label(statement.body.as_ref(), labels);
                labels.pop();
                duplicate
            }
            _ => None,
        }
    }

    fn first_duplicate_named_span(&self, names: &[NamedSpan]) -> Option<NamedSpan> {
        let mut seen = HashSet::with_capacity(names.len());
        for name in names {
            if !seen.insert(name.name.as_str()) {
                return Some(name.clone());
            }
        }
        None
    }

    fn first_duplicate_variable_declaration_name(
        &self,
        declaration: &VariableDeclaration,
    ) -> Option<NamedSpan> {
        let mut names = Vec::new();
        self.append_variable_declaration_bound_names(declaration, &mut names);
        self.first_duplicate_named_span(&names)
    }

    fn variable_declaration_names_conflict_with_statement_vars(
        &self,
        declaration: &VariableDeclaration,
        statement: &Statement,
    ) -> bool {
        let mut declaration_names = Vec::new();
        self.append_variable_declaration_bound_names(declaration, &mut declaration_names);
        let body_var_names = self.statement_var_declared_names(statement, StatementListKind::Block);
        let body_var_name_set: HashSet<_> = body_var_names
            .iter()
            .map(|name| name.name.as_str())
            .collect();
        declaration_names
            .iter()
            .any(|name| body_var_name_set.contains(name.name.as_str()))
    }

    fn statement_var_declared_names(
        &self,
        statement: &Statement,
        kind: StatementListKind,
    ) -> Vec<NamedSpan> {
        let mut names = Vec::new();
        self.append_statement_var_declared_names(statement, kind, &mut names);
        names
    }

    fn first_parameter_body_lexical_conflict(
        &self,
        params: &[Pattern],
        body: &[Statement],
    ) -> Option<NamedSpan> {
        let mut parameter_names = Vec::new();
        for param in params {
            Self::append_pattern_bound_names(param, &mut parameter_names);
        }
        let lexical_names =
            self.statement_list_lexically_declared_names(body, StatementListKind::FunctionBody);
        let lexical_name_set: HashSet<_> = lexical_names
            .iter()
            .map(|name| name.name.as_str())
            .collect();
        parameter_names
            .into_iter()
            .find(|param| lexical_name_set.contains(param.name.as_str()))
    }

    fn is_logical_and_or_expression(&self, expression: &Expression) -> bool {
        matches!(
            expression,
            Expression::Logical(expression)
                if matches!(expression.operator, LogicalOperator::And | LogicalOperator::Or)
        )
    }

    fn is_nullish_coalescing_expression(&self, expression: &Expression) -> bool {
        matches!(
            expression,
            Expression::Logical(expression)
                if expression.operator == LogicalOperator::NullishCoalescing
        )
    }

    fn is_parenthesized_expression(&self, expression: &Expression) -> bool {
        self.parenthesized_expressions.contains(&expression.span())
    }

    fn statement_list_lexically_declared_names(
        &self,
        statements: &[Statement],
        kind: StatementListKind,
    ) -> Vec<NamedSpan> {
        self.statement_list_lexically_declared_entries(statements, kind)
            .into_iter()
            .map(|entry| entry.binding)
            .collect()
    }

    fn statement_list_lexically_declared_entries(
        &self,
        statements: &[Statement],
        kind: StatementListKind,
    ) -> Vec<LexicalName> {
        let mut names = Vec::new();
        for statement in statements {
            self.append_statement_lexically_declared_names(statement, kind, &mut names);
        }
        names
    }

    fn append_statement_lexically_declared_names(
        &self,
        statement: &Statement,
        kind: StatementListKind,
        names: &mut Vec<LexicalName>,
    ) {
        match statement {
            Statement::Labeled(statement) => {
                self.append_statement_lexically_declared_names(statement.body.as_ref(), kind, names)
            }
            Statement::ImportDeclaration(declaration) if kind == StatementListKind::Module => {
                if let Some(clause) = &declaration.clause {
                    let mut bindings = Vec::new();
                    self.append_import_clause_bound_names(clause, &mut bindings);
                    names.extend(bindings.into_iter().map(|binding| LexicalName {
                        binding,
                        function_declaration: false,
                    }));
                }
            }
            Statement::ExportDeclaration(declaration) => match declaration {
                ExportDeclaration::Default(declaration) => match &declaration.declaration {
                    ExportDefaultKind::Function(function) => {
                        if let Some(identifier) = &function.id {
                            names.push(LexicalName {
                                binding: NamedSpan {
                                    name: identifier.name.clone(),
                                    span: identifier.span,
                                },
                                function_declaration: !function.is_async && !function.is_generator,
                            });
                        }
                    }
                    ExportDefaultKind::Class(class) => {
                        if let Some(identifier) = &class.id {
                            names.push(LexicalName {
                                binding: NamedSpan {
                                    name: identifier.name.clone(),
                                    span: identifier.span,
                                },
                                function_declaration: false,
                            });
                        }
                    }
                    ExportDefaultKind::Expression(_) => {}
                },
                ExportDeclaration::Declaration(declaration) => {
                    self.append_exported_declaration_lexical_names(declaration, kind, names);
                }
                ExportDeclaration::All(_) | ExportDeclaration::Named(_) => {}
            },
            Statement::VariableDeclaration(declaration)
                if declaration.kind != VariableKind::Var =>
            {
                let mut bindings = Vec::new();
                self.append_variable_declaration_bound_names(declaration, &mut bindings);
                names.extend(bindings.into_iter().map(|binding| LexicalName {
                    binding,
                    function_declaration: false,
                }));
            }
            Statement::FunctionDeclaration(function)
                if kind.function_declarations_are_lexical() =>
            {
                if let Some(identifier) = &function.id {
                    names.push(LexicalName {
                        binding: NamedSpan {
                            name: identifier.name.clone(),
                            span: identifier.span,
                        },
                        function_declaration: !function.is_async && !function.is_generator,
                    });
                }
            }
            Statement::ClassDeclaration(class) => {
                if let Some(identifier) = &class.id {
                    names.push(LexicalName {
                        binding: NamedSpan {
                            name: identifier.name.clone(),
                            span: identifier.span,
                        },
                        function_declaration: false,
                    });
                }
            }
            _ => {}
        }
    }

    fn append_exported_declaration_lexical_names(
        &self,
        declaration: &ExportedDeclaration,
        kind: StatementListKind,
        names: &mut Vec<LexicalName>,
    ) {
        match declaration {
            ExportedDeclaration::Variable(declaration) if declaration.kind != VariableKind::Var => {
                let mut bindings = Vec::new();
                self.append_variable_declaration_bound_names(declaration, &mut bindings);
                names.extend(bindings.into_iter().map(|binding| LexicalName {
                    binding,
                    function_declaration: false,
                }));
            }
            ExportedDeclaration::Function(function) if kind.function_declarations_are_lexical() => {
                if let Some(identifier) = &function.id {
                    names.push(LexicalName {
                        binding: NamedSpan {
                            name: identifier.name.clone(),
                            span: identifier.span,
                        },
                        function_declaration: !function.is_async && !function.is_generator,
                    });
                }
            }
            ExportedDeclaration::Class(class) => {
                if let Some(identifier) = &class.id {
                    names.push(LexicalName {
                        binding: NamedSpan {
                            name: identifier.name.clone(),
                            span: identifier.span,
                        },
                        function_declaration: false,
                    });
                }
            }
            _ => {}
        }
    }

    fn statement_list_var_declared_names(
        &self,
        statements: &[Statement],
        kind: StatementListKind,
    ) -> Vec<NamedSpan> {
        let mut names = Vec::new();
        for statement in statements {
            self.append_statement_var_declared_names(statement, kind, &mut names);
        }
        names
    }

    fn append_statement_var_declared_names(
        &self,
        statement: &Statement,
        kind: StatementListKind,
        names: &mut Vec<NamedSpan>,
    ) {
        match statement {
            Statement::Directive(_)
            | Statement::Empty(_)
            | Statement::Debugger(_)
            | Statement::ImportDeclaration(_)
            | Statement::Expression(_)
            | Statement::Return(_)
            | Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Throw(_)
            | Statement::ClassDeclaration(_) => {}
            Statement::Labeled(statement) => {
                self.append_statement_var_declared_names(statement.body.as_ref(), kind, names)
            }
            Statement::ExportDeclaration(declaration) => {
                if let ExportDeclaration::Declaration(ExportedDeclaration::Variable(declaration)) =
                    declaration
                    && declaration.kind == VariableKind::Var
                {
                    self.append_variable_declaration_bound_names(declaration, names);
                }
            }
            Statement::VariableDeclaration(declaration)
                if declaration.kind == VariableKind::Var =>
            {
                self.append_variable_declaration_bound_names(declaration, names);
            }
            Statement::VariableDeclaration(_) => {}
            Statement::FunctionDeclaration(function) if kind.function_declarations_are_var() => {
                if let Some(identifier) = &function.id {
                    names.push(NamedSpan {
                        name: identifier.name.clone(),
                        span: identifier.span,
                    });
                }
            }
            Statement::FunctionDeclaration(_) => {}
            Statement::Block(block) => {
                for statement in &block.body {
                    self.append_statement_var_declared_names(
                        statement,
                        StatementListKind::Block,
                        names,
                    );
                }
            }
            Statement::If(statement) => {
                self.append_statement_var_declared_names(
                    statement.consequent.as_ref(),
                    StatementListKind::Block,
                    names,
                );
                if let Some(alternate) = &statement.alternate {
                    self.append_statement_var_declared_names(
                        alternate.as_ref(),
                        StatementListKind::Block,
                        names,
                    );
                }
            }
            Statement::While(statement) => {
                self.append_statement_var_declared_names(
                    statement.body.as_ref(),
                    StatementListKind::Block,
                    names,
                );
            }
            Statement::DoWhile(statement) => {
                self.append_statement_var_declared_names(
                    statement.body.as_ref(),
                    StatementListKind::Block,
                    names,
                );
            }
            Statement::For(statement) => match statement {
                ForStatement::Classic(statement) => {
                    if let Some(ForInit::VariableDeclaration(declaration)) = &statement.init
                        && declaration.kind == VariableKind::Var
                    {
                        self.append_variable_declaration_bound_names(declaration, names);
                    }
                    self.append_statement_var_declared_names(
                        statement.body.as_ref(),
                        StatementListKind::Block,
                        names,
                    );
                }
                ForStatement::In(statement) | ForStatement::Of(statement) => {
                    if let ForLeft::VariableDeclaration(declaration) = &statement.left
                        && declaration.kind == VariableKind::Var
                    {
                        self.append_variable_declaration_bound_names(declaration, names);
                    }
                    self.append_statement_var_declared_names(
                        statement.body.as_ref(),
                        StatementListKind::Block,
                        names,
                    );
                }
            },
            Statement::Switch(statement) => {
                for case in &statement.cases {
                    for statement in &case.consequent {
                        self.append_statement_var_declared_names(
                            statement,
                            StatementListKind::SwitchCase,
                            names,
                        );
                    }
                }
            }
            Statement::Try(statement) => {
                for statement in &statement.block.body {
                    self.append_statement_var_declared_names(
                        statement,
                        StatementListKind::Block,
                        names,
                    );
                }
                if let Some(handler) = &statement.handler {
                    for statement in &handler.body.body {
                        self.append_statement_var_declared_names(
                            statement,
                            StatementListKind::Block,
                            names,
                        );
                    }
                }
                if let Some(finalizer) = &statement.finalizer {
                    for statement in &finalizer.body {
                        self.append_statement_var_declared_names(
                            statement,
                            StatementListKind::Block,
                            names,
                        );
                    }
                }
            }
            Statement::With(statement) => {
                self.append_statement_var_declared_names(
                    statement.body.as_ref(),
                    StatementListKind::Block,
                    names,
                );
            }
        }
    }

    fn append_statement_exported_names(&self, statement: &Statement, names: &mut Vec<NamedSpan>) {
        let Statement::ExportDeclaration(declaration) = statement else {
            return;
        };
        match declaration {
            ExportDeclaration::All(declaration) => {
                if let Some(exported) = &declaration.exported {
                    self.append_module_export_name(exported, names);
                }
            }
            ExportDeclaration::Named(declaration) => {
                for specifier in &declaration.specifiers {
                    self.append_module_export_name(&specifier.exported, names);
                }
            }
            ExportDeclaration::Default(declaration) => names.push(NamedSpan {
                name: "default".to_string(),
                span: declaration.span,
            }),
            ExportDeclaration::Declaration(declaration) => match declaration {
                ExportedDeclaration::Variable(declaration) => {
                    self.append_variable_declaration_bound_names(declaration, names);
                }
                ExportedDeclaration::Function(function) => {
                    if let Some(identifier) = &function.id {
                        names.push(NamedSpan {
                            name: identifier.name.clone(),
                            span: identifier.span,
                        });
                    }
                }
                ExportedDeclaration::Class(class) => {
                    if let Some(identifier) = &class.id {
                        names.push(NamedSpan {
                            name: identifier.name.clone(),
                            span: identifier.span,
                        });
                    }
                }
            },
        }
    }

    fn append_import_clause_bound_names(&self, clause: &ImportClause, names: &mut Vec<NamedSpan>) {
        match clause {
            ImportClause::Default(identifier) => names.push(NamedSpan {
                name: identifier.name.clone(),
                span: identifier.span,
            }),
            ImportClause::Namespace { default, namespace } => {
                if let Some(identifier) = default {
                    names.push(NamedSpan {
                        name: identifier.name.clone(),
                        span: identifier.span,
                    });
                }
                names.push(NamedSpan {
                    name: namespace.name.clone(),
                    span: namespace.span,
                });
            }
            ImportClause::Named {
                default,
                specifiers,
            } => {
                if let Some(identifier) = default {
                    names.push(NamedSpan {
                        name: identifier.name.clone(),
                        span: identifier.span,
                    });
                }
                for specifier in specifiers {
                    names.push(NamedSpan {
                        name: specifier.local.name.clone(),
                        span: specifier.local.span,
                    });
                }
            }
        }
    }

    fn append_module_export_name(&self, name: &ModuleExportName, names: &mut Vec<NamedSpan>) {
        match name {
            ModuleExportName::Identifier(identifier) => names.push(NamedSpan {
                name: identifier.name.clone(),
                span: identifier.span,
            }),
            ModuleExportName::String(literal) => names.push(NamedSpan {
                name: literal.value.clone(),
                span: literal.span,
            }),
        }
    }

    fn append_variable_declaration_bound_names(
        &self,
        declaration: &VariableDeclaration,
        names: &mut Vec<NamedSpan>,
    ) {
        for declarator in &declaration.declarations {
            Self::append_pattern_bound_names(&declarator.pattern, names);
        }
    }

    fn append_pattern_bound_names(pattern: &Pattern, names: &mut Vec<NamedSpan>) {
        match pattern {
            Pattern::Identifier(identifier) => names.push(NamedSpan {
                name: identifier.name.clone(),
                span: identifier.span,
            }),
            Pattern::Array(pattern) => {
                for element in pattern.elements.iter().flatten() {
                    Self::append_pattern_bound_names(element, names);
                }
            }
            Pattern::Object(pattern) => {
                for property in &pattern.properties {
                    match property {
                        ObjectPatternProperty::Property { value, .. } => {
                            Self::append_pattern_bound_names(value, names);
                        }
                        ObjectPatternProperty::Rest { argument, .. } => {
                            Self::append_pattern_bound_names(argument, names);
                        }
                    }
                }
            }
            Pattern::Rest(pattern) => {
                Self::append_pattern_bound_names(pattern.argument.as_ref(), names)
            }
            Pattern::Assignment(pattern) => {
                Self::append_pattern_bound_names(pattern.left.as_ref(), names);
            }
        }
    }

    fn validate_export_declaration(
        &self,
        declaration: &ExportDeclaration,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        match declaration {
            ExportDeclaration::All(declaration) => {
                if let Some(attributes) = &declaration.attributes {
                    self.validate_module_attributes(attributes)?;
                    self.validate_expression(attributes, context, private_names)?;
                }
            }
            ExportDeclaration::Named(declaration) => {
                if let Some(attributes) = &declaration.attributes {
                    self.validate_module_attributes(attributes)?;
                    self.validate_expression(attributes, context, private_names)?;
                }
            }
            ExportDeclaration::Default(declaration) => match &declaration.declaration {
                ExportDefaultKind::Function(function) => {
                    self.validate_function(function, context, private_names)?
                }
                ExportDefaultKind::Class(class) => {
                    self.validate_class(class, context, private_names)?
                }
                ExportDefaultKind::Expression(expression) => {
                    self.validate_expression(expression, context, private_names)?
                }
            },
            ExportDeclaration::Declaration(declaration) => match declaration {
                ExportedDeclaration::Variable(declaration) => {
                    self.validate_variable_declaration(declaration, context, private_names)?
                }
                ExportedDeclaration::Function(function) => {
                    self.validate_function(function, context, private_names)?
                }
                ExportedDeclaration::Class(class) => {
                    self.validate_class(class, context, private_names)?
                }
            },
        }
        Ok(())
    }

    fn validate_module_export_early_errors(&self, program: &Program) -> Result<(), ParseError> {
        let mut declared_names = HashSet::new();
        for name in self.statement_list_var_declared_names(&program.body, StatementListKind::Module)
        {
            declared_names.insert(name.name);
        }
        for name in
            self.statement_list_lexically_declared_names(&program.body, StatementListKind::Module)
        {
            declared_names.insert(name.name);
        }

        for statement in &program.body {
            let Statement::ExportDeclaration(declaration) = statement else {
                continue;
            };
            if let ExportDeclaration::Named(named) = declaration {
                if named.source.is_some() {
                    continue;
                }
                for specifier in &named.specifiers {
                    match &specifier.local {
                        ModuleExportName::Identifier(identifier) => {
                            if !declared_names.contains(&identifier.name) {
                                return Err(ParseError::new(
                                    "exported binding is not declared in this module",
                                    identifier.span,
                                ));
                            }
                        }
                        ModuleExportName::String(literal) => {
                            return Err(ParseError::new(
                                "local export bindings must be identifiers",
                                literal.span,
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_module_attributes(&self, attributes: &Expression) -> Result<(), ParseError> {
        let Expression::Object(object) = attributes else {
            return Ok(());
        };
        let mut seen = HashSet::new();
        for property in &object.properties {
            let ObjectProperty::Property { key, .. } = property else {
                continue;
            };
            let Some(name) = self.property_key_name(key) else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                return Err(ParseError::new(
                    "duplicate import/export attribute key",
                    key.span(),
                ));
            }
        }
        Ok(())
    }

    fn validate_for_statement(
        &self,
        statement: &ForStatement,
        context: ValidationContext,
        private_names: &HashSet<String>,
        list_kind: StatementListKind,
    ) -> Result<(), ParseError> {
        match statement {
            ForStatement::Classic(statement) => {
                if let Some(init) = &statement.init {
                    match init {
                        ForInit::VariableDeclaration(declaration) => {
                            if declaration.kind != VariableKind::Var
                                && self.variable_declaration_names_conflict_with_statement_vars(
                                    declaration,
                                    statement.body.as_ref(),
                                )
                            {
                                return Err(ParseError::new(
                                    "for declaration conflicts with var declaration in body",
                                    declaration.span,
                                ));
                            }
                            self.validate_variable_declaration(declaration, context, private_names)?
                        }
                        ForInit::Expression(expression) => {
                            self.validate_expression(expression, context, private_names)?
                        }
                    }
                }
                if let Some(test) = &statement.test {
                    self.validate_expression(test, context, private_names)?;
                }
                if let Some(update) = &statement.update {
                    self.validate_expression(update, context, private_names)?;
                }
                self.validate_statement_position(statement.body.as_ref(), false)?;
                self.validate_statement(statement.body.as_ref(), context, private_names, list_kind)
            }
            ForStatement::In(statement) => {
                match &statement.left {
                    ForLeft::VariableDeclaration(declaration) => {
                        if declaration.kind != VariableKind::Var
                            && let Some(duplicate) =
                                self.first_duplicate_variable_declaration_name(declaration)
                        {
                            return Err(ParseError::new(
                                "duplicate binding name in for-in/of declaration",
                                duplicate.span,
                            ));
                        }
                        if declaration.kind == VariableKind::Var
                            && declaration
                                .declarations
                                .iter()
                                .any(|item| item.init.is_some())
                            && (context.strict
                                || declaration
                                    .declarations
                                    .iter()
                                    .any(|item| !matches!(item.pattern, Pattern::Identifier(_))))
                        {
                            return Err(ParseError::new(
                                "for-in var initializers require a simple identifier in sloppy mode",
                                declaration.span,
                            ));
                        }
                        if declaration.kind != VariableKind::Var
                            && self.variable_declaration_names_conflict_with_statement_vars(
                                declaration,
                                statement.body.as_ref(),
                            )
                        {
                            return Err(ParseError::new(
                                "for-in/of declaration conflicts with var declaration in body",
                                declaration.span,
                            ));
                        }
                        self.validate_variable_declaration(declaration, context, private_names)?;
                    }
                    ForLeft::Pattern(pattern) => {
                        self.validate_pattern(pattern, context, private_names)?
                    }
                    ForLeft::Expression(expression) => {
                        if !self.is_valid_for_each_left_target(expression, context.strict) {
                            return Err(ParseError::new(
                                "for-in/of left side must be a left-hand-side expression",
                                expression.span(),
                            ));
                        }
                        if matches!(expression, Expression::Array(_) | Expression::Object(_)) {
                            self.validate_assignment_target_expression(
                                expression,
                                context,
                                private_names,
                            )?
                        } else {
                            self.validate_expression(expression, context, private_names)?
                        }
                    }
                }
                self.validate_expression(&statement.right, context, private_names)?;
                self.validate_statement_position(statement.body.as_ref(), false)?;
                self.validate_statement(statement.body.as_ref(), context, private_names, list_kind)
            }
            ForStatement::Of(statement) => {
                if matches!(statement.right, Expression::Sequence(_))
                    && !self.is_parenthesized_expression(&statement.right)
                {
                    return Err(ParseError::new(
                        "for-of right-hand side must be a single assignment expression",
                        statement.right.span(),
                    ));
                }
                match &statement.left {
                    ForLeft::VariableDeclaration(declaration) => {
                        if declaration.kind != VariableKind::Var
                            && let Some(duplicate) =
                                self.first_duplicate_variable_declaration_name(declaration)
                        {
                            return Err(ParseError::new(
                                "duplicate binding name in for-in/of declaration",
                                duplicate.span,
                            ));
                        }
                        if declaration.kind != VariableKind::Var
                            && self.variable_declaration_names_conflict_with_statement_vars(
                                declaration,
                                statement.body.as_ref(),
                            )
                        {
                            return Err(ParseError::new(
                                "for-in/of declaration conflicts with var declaration in body",
                                declaration.span,
                            ));
                        }
                        self.validate_variable_declaration(declaration, context, private_names)?
                    }
                    ForLeft::Pattern(pattern) => {
                        self.validate_pattern(pattern, context, private_names)?
                    }
                    ForLeft::Expression(expression) => {
                        if !statement.is_await
                            && matches!(
                                expression,
                                Expression::Identifier(identifier)
                                    if identifier.name == "async"
                                        && !self.identifier_was_escaped(identifier)
                            )
                            && !self.is_parenthesized_expression(expression)
                        {
                            return Err(ParseError::new(
                                "for-of left-hand side cannot start with 'async'",
                                expression.span(),
                            ));
                        }
                        if !self.is_valid_for_each_left_target(expression, context.strict) {
                            return Err(ParseError::new(
                                "for-in/of left side must be a left-hand-side expression",
                                expression.span(),
                            ));
                        }
                        if matches!(expression, Expression::Array(_) | Expression::Object(_)) {
                            self.validate_assignment_target_expression(
                                expression,
                                context,
                                private_names,
                            )?
                        } else {
                            self.validate_expression(expression, context, private_names)?
                        }
                    }
                }
                self.validate_expression(&statement.right, context, private_names)?;
                self.validate_statement_position(statement.body.as_ref(), false)?;
                self.validate_statement(statement.body.as_ref(), context, private_names, list_kind)
            }
        }
    }

    fn validate_statement_position(
        &self,
        statement: &Statement,
        allow_function_declaration: bool,
    ) -> Result<(), ParseError> {
        match statement {
            statement if Self::is_labelled_function(statement) => Err(ParseError::new(
                "declarations are not allowed in statement position",
                statement.span(),
            )),
            Statement::VariableDeclaration(declaration)
                if declaration.kind != VariableKind::Var
                    && !self.statement_position_allows_linebreak_let(declaration) =>
            {
                Err(ParseError::new(
                    "declarations are not allowed in statement position",
                    declaration.span,
                ))
            }
            Statement::FunctionDeclaration(function)
                if !allow_function_declaration || function.is_async || function.is_generator =>
            {
                Err(ParseError::new(
                    "declarations are not allowed in statement position",
                    function.span,
                ))
            }
            Statement::ClassDeclaration(class) => Err(ParseError::new(
                "declarations are not allowed in statement position",
                class.span,
            )),
            Statement::ImportDeclaration(declaration) => Err(ParseError::new(
                "declarations are not allowed in statement position",
                declaration.span,
            )),
            Statement::ExportDeclaration(declaration) => Err(ParseError::new(
                "declarations are not allowed in statement position",
                declaration.span(),
            )),
            _ => Ok(()),
        }
    }

    fn is_labelled_function(statement: &Statement) -> bool {
        match statement {
            Statement::Labeled(statement) => {
                matches!(statement.body.as_ref(), Statement::FunctionDeclaration(_))
                    || Self::is_labelled_function(statement.body.as_ref())
            }
            _ => false,
        }
    }

    fn statement_position_allows_linebreak_let(&self, declaration: &VariableDeclaration) -> bool {
        declaration.kind == VariableKind::Let
            && declaration.declarations.first().is_some_and(|item| {
                matches!(item.pattern, Pattern::Identifier(_) | Pattern::Object(_))
                    && item.pattern.span().start.line > declaration.span.start.line
            })
    }

    fn validate_variable_declaration(
        &self,
        declaration: &VariableDeclaration,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        for declarator in &declaration.declarations {
            self.validate_pattern(&declarator.pattern, context, private_names)?;
            if let Some(init) = &declarator.init {
                self.validate_expression(init, context, private_names)?;
            }
        }
        Ok(())
    }

    fn validate_function(
        &self,
        function: &Function,
        inherited_context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        let has_use_strict = self.statement_list_has_use_strict(&function.body.body);
        let context = inherited_context.for_function(
            function.is_async,
            function.is_generator,
            has_use_strict,
        );
        let name_context = inherited_context
            .with_strict(inherited_context.strict || has_use_strict)
            .for_function_name(false, function.is_async, function.is_generator);

        self.validate_function_core(function, name_context, context, false, private_names)
    }

    fn validate_function_expression(
        &self,
        function: &Function,
        inherited_context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        let has_use_strict = self.statement_list_has_use_strict(&function.body.body);
        let context = inherited_context.for_function(
            function.is_async,
            function.is_generator,
            has_use_strict,
        );
        let name_context = inherited_context
            .with_strict(inherited_context.strict || has_use_strict)
            .for_function_name(true, function.is_async, function.is_generator);

        self.validate_function_core(function, name_context, context, false, private_names)
    }

    fn validate_function_core(
        &self,
        function: &Function,
        name_context: ValidationContext,
        context: ValidationContext,
        reject_duplicate_parameters: bool,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        if let Some(identifier) = &function.id {
            self.validate_binding_identifier(identifier, name_context)?;
        }

        let simple = self.is_simple_parameter_list(&function.params);
        if self.statement_list_has_use_strict(&function.body.body) && !simple {
            return Err(ParseError::new(
                "use strict directives require a simple parameter list",
                function.body.span,
            ));
        }
        if function.is_async && self.patterns_contain_await_expression(&function.params) {
            return Err(ParseError::new(
                "await expressions are not allowed in formal parameters here",
                function.span,
            ));
        }
        if function.is_generator && self.patterns_contain_yield_expression(&function.params) {
            return Err(ParseError::new(
                "yield expressions are not allowed in formal parameters here",
                function.span,
            ));
        }
        if let Some(duplicate) = (reject_duplicate_parameters || context.strict || !simple)
            .then(|| self.first_duplicate_bound_name(&function.params))
            .flatten()
        {
            return Err(ParseError::new("duplicate parameter name", duplicate.span));
        }
        if let Some(conflict) =
            self.first_parameter_body_lexical_conflict(&function.params, &function.body.body)
        {
            return Err(ParseError::new(
                "parameter name conflicts with lexical declaration in function body",
                conflict.span,
            ));
        }

        for param in &function.params {
            self.validate_pattern(param, context, private_names)?;
        }
        self.validate_statement_list_control(&function.body.body, context, &[], false, false)?;
        self.validate_statement_list(
            &function.body.body,
            context,
            private_names,
            StatementListKind::FunctionBody,
        )
    }

    fn validate_method_function(
        &self,
        function: &Function,
        inherited_context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        let has_use_strict = self.statement_list_has_use_strict(&function.body.body);
        let context = inherited_context
            .for_function(function.is_async, function.is_generator, has_use_strict)
            .with_super_capabilities(
                inherited_context.super_property_allowed,
                inherited_context.super_call_allowed,
            );
        let name_context = inherited_context
            .with_strict(inherited_context.strict || has_use_strict)
            .for_function_name(true, function.is_async, function.is_generator);

        self.validate_function_core(function, name_context, context, true, private_names)
    }

    fn validate_arrow_function(
        &self,
        function: &ArrowFunction,
        inherited_context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        let has_use_strict = matches!(&function.body, ArrowBody::Block(block) if self.statement_list_has_use_strict(&block.body));
        let param_context = inherited_context.for_arrow_function(function.is_async, has_use_strict);
        let body_context = param_context.with_class_static_block(false);
        let simple = self.is_simple_parameter_list(&function.params);

        if has_use_strict && !simple {
            return Err(ParseError::new(
                "use strict directives require a simple parameter list",
                function.span,
            ));
        }
        if function.is_async && self.patterns_contain_await_expression(&function.params) {
            return Err(ParseError::new(
                "await expressions are not allowed in formal parameters here",
                function.span,
            ));
        }
        if let Some(duplicate) = self.first_duplicate_bound_name(&function.params) {
            return Err(ParseError::new("duplicate parameter name", duplicate.span));
        }
        if let ArrowBody::Block(block) = &function.body
            && let Some(conflict) =
                self.first_parameter_body_lexical_conflict(&function.params, &block.body)
        {
            return Err(ParseError::new(
                "parameter name conflicts with lexical declaration in function body",
                conflict.span,
            ));
        }

        for param in &function.params {
            self.validate_pattern(param, param_context, private_names)?;
        }
        match &function.body {
            ArrowBody::Expression(expression) => {
                self.validate_expression(expression, body_context, private_names)
            }
            ArrowBody::Block(block) => {
                self.validate_statement_list_control(&block.body, body_context, &[], false, false)?;
                self.validate_statement_list(
                    &block.body,
                    body_context,
                    private_names,
                    StatementListKind::FunctionBody,
                )
            }
        }
    }

    fn validate_class(
        &self,
        class: &Class,
        context: ValidationContext,
        outer_private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        let strict_context = context.with_strict(true);
        let class_element_context = context.for_class_element_body();
        let static_block_context = class_element_context.with_class_static_block(true);
        let mut class_private_names = outer_private_names.clone();
        class_private_names.extend(self.collect_class_private_names(class)?);
        if let Some(identifier) = &class.id {
            self.validate_binding_identifier(identifier, strict_context)?;
        }
        for decorator in &class.decorators {
            self.validate_expression(decorator, context, outer_private_names)?;
        }
        if let Some(super_class) = &class.super_class {
            self.validate_expression(super_class, strict_context, outer_private_names)?;
        }

        for element in &class.body {
            match element {
                ClassElement::Empty(_) => {}
                ClassElement::StaticBlock(block) => {
                    if self.statement_list_contains_super_call(&block.body) {
                        return Err(ParseError::new("super() is not allowed here", block.span));
                    }
                    self.validate_statement_list_control(
                        &block.body,
                        static_block_context,
                        &[],
                        false,
                        false,
                    )?;
                    self.validate_statement_list(
                        &block.body,
                        static_block_context,
                        &class_private_names,
                        StatementListKind::StaticBlock,
                    )?;
                }
                ClassElement::Method(method) => {
                    for decorator in &method.decorators {
                        self.validate_expression(decorator, context, &class_private_names)?;
                    }
                    self.validate_property_key(&method.key, strict_context, &class_private_names)?;
                    if !method.is_static
                        && self.is_constructor_key(&method.key)
                        && method.kind != MethodKind::Constructor
                    {
                        return Err(ParseError::new(
                            "constructors cannot be async, generators, getters, or setters",
                            method.span,
                        ));
                    }
                    match method.kind {
                        MethodKind::Getter if !method.value.params.is_empty() => {
                            return Err(ParseError::new(
                                "getter methods must not declare parameters",
                                method.value.span,
                            ));
                        }
                        MethodKind::Setter
                            if method.value.params.len() != 1
                                || matches!(method.value.params[0], Pattern::Rest(_)) =>
                        {
                            return Err(ParseError::new(
                                "setter methods must declare exactly one non-rest parameter",
                                method.value.span,
                            ));
                        }
                        _ => {}
                    }
                    let method_context = strict_context.with_super_capabilities(
                        true,
                        method.kind == MethodKind::Constructor && class.super_class.is_some(),
                    );
                    self.validate_method_function(
                        &method.value,
                        method_context,
                        &class_private_names,
                    )?;
                }
                ClassElement::Field(field) => {
                    for decorator in &field.decorators {
                        self.validate_expression(decorator, context, &class_private_names)?;
                    }
                    self.validate_property_key(&field.key, strict_context, &class_private_names)?;
                    if let Some(value) = &field.value {
                        if self.expression_contains_arguments(value) {
                            return Err(ParseError::new(
                                "class field initializers cannot contain 'arguments'",
                                value.span(),
                            ));
                        }
                        if self.expression_contains_super_call(value) {
                            return Err(ParseError::new(
                                "super() is not allowed in class field initializers",
                                value.span(),
                            ));
                        }
                        self.validate_expression(
                            value,
                            class_element_context,
                            &class_private_names,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_property_key(
        &self,
        key: &PropertyKey,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        if let PropertyKey::Computed { expression, .. } = key {
            self.validate_expression(expression, context, private_names)?;
        }
        Ok(())
    }

    fn validate_literal(
        &self,
        literal: &Literal,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        match literal {
            Literal::Number(number) => {
                if context.strict && self.is_strict_legacy_decimal_literal(&number.raw) {
                    return Err(ParseError::new(
                        "legacy decimal literals are not allowed in strict mode",
                        number.span,
                    ));
                }
                if self.numeric_literal_followed_by_identifier(number.span) {
                    return Err(ParseError::new(
                        "numeric literals must not be immediately followed by identifier text",
                        number.span,
                    ));
                }
                Ok(())
            }
            Literal::String(string) => {
                if context.strict
                    && self
                        .source_slice(string.span)
                        .is_some_and(|raw| self.raw_string_has_legacy_escape(raw))
                {
                    return Err(ParseError::new(
                        "legacy string escapes are not allowed in strict mode",
                        string.span,
                    ));
                }
                Ok(())
            }
            Literal::Template(template) => self.validate_template_literal(template, context),
            _ => Ok(()),
        }
    }

    fn validate_object_method(
        &self,
        function: &Function,
        kind: MethodKind,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        match kind {
            MethodKind::Getter if !function.params.is_empty() => {
                return Err(ParseError::new(
                    "getter methods must not declare parameters",
                    function.span,
                ));
            }
            MethodKind::Setter
                if function.params.len() != 1 || matches!(function.params[0], Pattern::Rest(_)) =>
            {
                return Err(ParseError::new(
                    "setter methods must declare exactly one non-rest parameter",
                    function.span,
                ));
            }
            _ => {}
        }
        let method_context = context.with_super_capabilities(true, false);
        self.validate_method_function(function, method_context, private_names)
    }

    fn validate_pattern(
        &self,
        pattern: &Pattern,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        match pattern {
            Pattern::Identifier(identifier) => {
                self.validate_binding_identifier(identifier, context)
            }
            Pattern::Array(pattern) => {
                for element in pattern.elements.iter().flatten() {
                    self.validate_pattern(element, context, private_names)?;
                }
                Ok(())
            }
            Pattern::Object(pattern) => {
                for property in &pattern.properties {
                    match property {
                        ObjectPatternProperty::Property { key, value, .. } => {
                            self.validate_property_key(key, context, private_names)?;
                            self.validate_pattern(value, context, private_names)?;
                        }
                        ObjectPatternProperty::Rest { argument, .. } => {
                            self.validate_pattern(argument, context, private_names)?;
                        }
                    }
                }
                Ok(())
            }
            Pattern::Rest(pattern) => {
                self.validate_pattern(pattern.argument.as_ref(), context, private_names)
            }
            Pattern::Assignment(pattern) => {
                self.validate_pattern(pattern.left.as_ref(), context, private_names)?;
                self.validate_expression(&pattern.right, context, private_names)
            }
        }
    }

    fn validate_binding_identifier(
        &self,
        identifier: &Identifier,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        let name = self
            .decoded_identifier_name(identifier)
            .unwrap_or_else(|| identifier.name.clone());
        self.validate_identifier_reference(identifier, context)?;
        if context.in_class_static_block && matches!(name.as_str(), "await" | "arguments") {
            return Err(ParseError::new(
                format!("identifier '{}' is reserved here", name),
                identifier.span,
            ));
        }
        if context.strict && matches!(name.as_str(), "eval" | "arguments") {
            return Err(ParseError::new(
                format!("identifier '{}' is reserved in strict mode", name),
                identifier.span,
            ));
        }
        Ok(())
    }

    fn validate_expression(
        &self,
        expression: &Expression,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        match expression {
            Expression::Identifier(identifier) => {
                self.validate_identifier_reference(identifier, context)
            }
            Expression::PrivateIdentifier(identifier) => {
                self.validate_private_name(identifier, private_names)
            }
            Expression::Literal(literal) => self.validate_literal(literal, context),
            Expression::This(_) | Expression::Super(_) => Ok(()),
            Expression::MetaProperty(expression) => {
                if expression.meta.name == "new"
                    && expression.property.name == "target"
                    && self.identifier_was_escaped(&expression.property)
                {
                    return Err(ParseError::new(
                        "escaped 'target' is not allowed in new.target",
                        expression.property.span,
                    ));
                }
                if expression.meta.name == "new"
                    && expression.property.name == "target"
                    && !context.new_target_allowed
                {
                    return Err(ParseError::new(
                        "new.target is only allowed inside functions or class static blocks",
                        expression.span,
                    ));
                }
                Ok(())
            }
            Expression::Array(expression) => {
                for element in expression.elements.iter().flatten() {
                    match element {
                        ArrayElement::Expression(expression) => {
                            self.validate_expression(expression, context, private_names)?
                        }
                        ArrayElement::Spread { argument, .. } => {
                            self.validate_expression(argument, context, private_names)?
                        }
                    }
                }
                Ok(())
            }
            Expression::Object(expression) => {
                let mut proto_property_count = 0usize;
                for property in &expression.properties {
                    match property {
                        ObjectProperty::Property {
                            key,
                            value,
                            kind,
                            shorthand,
                            ..
                        } => {
                            self.validate_property_key(key, context, private_names)?;
                            if *kind == ObjectPropertyKind::Init
                                && !shorthand
                                && self.is_proto_data_property_name(key)
                            {
                                proto_property_count += 1;
                                if proto_property_count > 1 {
                                    return Err(ParseError::new(
                                        "duplicate __proto__ property in object literal",
                                        key.span(),
                                    ));
                                }
                            }
                            if *kind != ObjectPropertyKind::Init {
                                let Expression::Function(function) = value else {
                                    return Err(ParseError::new(
                                        "object methods must contain function bodies",
                                        value.span(),
                                    ));
                                };
                                self.validate_object_method(
                                    function,
                                    match kind {
                                        ObjectPropertyKind::Method => MethodKind::Method,
                                        ObjectPropertyKind::Getter => MethodKind::Getter,
                                        ObjectPropertyKind::Setter => MethodKind::Setter,
                                        ObjectPropertyKind::Init => unreachable!(),
                                    },
                                    context,
                                    private_names,
                                )?;
                            } else {
                                self.validate_expression(value, context, private_names)?;
                            }
                        }
                        ObjectProperty::Spread { argument, .. } => {
                            self.validate_expression(argument, context, private_names)?
                        }
                    }
                }
                Ok(())
            }
            Expression::Function(function) => {
                self.validate_function_expression(function, context, private_names)
            }
            Expression::ArrowFunction(function) => {
                self.validate_arrow_function(function, context, private_names)
            }
            Expression::Class(class) => self.validate_class(class, context, private_names),
            Expression::TaggedTemplate(expression) => {
                self.validate_expression(&expression.tag, context, private_names)?;
                self.validate_template_literal(&expression.quasi, context)
            }
            Expression::Yield(expression) => {
                if context.in_class_static_block {
                    return Err(ParseError::new(
                        "yield is not allowed in class static blocks",
                        expression.span,
                    ));
                }
                if let Some(argument) = &expression.argument {
                    self.validate_expression(argument, context, private_names)?;
                }
                Ok(())
            }
            Expression::Await(expression) => {
                if context.in_class_static_block {
                    return Err(ParseError::new(
                        "await is not allowed in class static blocks",
                        expression.span,
                    ));
                }
                self.validate_expression(&expression.argument, context, private_names)
            }
            Expression::Unary(expression) => {
                self.validate_expression(&expression.argument, context, private_names)?;
                if expression.operator == UnaryOperator::Delete
                    && self.is_private_member_expression(&expression.argument)
                {
                    return Err(ParseError::new(
                        "private member expressions cannot be deleted",
                        expression.argument.span(),
                    ));
                }
                if expression.operator == UnaryOperator::Delete
                    && context.strict
                    && matches!(expression.argument, Expression::Identifier(_))
                {
                    return Err(ParseError::new(
                        "delete of an unqualified identifier is not allowed in strict mode",
                        expression.argument.span(),
                    ));
                }
                Ok(())
            }
            Expression::Update(expression) => {
                self.validate_expression(&expression.argument, context, private_names)?;
                if !self.is_valid_update_target(&expression.argument, context.strict) {
                    return Err(ParseError::new(
                        "invalid update target",
                        expression.argument.span(),
                    ));
                }
                self.validate_simple_target_identifier(&expression.argument, context)
            }
            Expression::Binary(expression) => {
                self.validate_expression(&expression.left, context, private_names)?;
                self.validate_expression(&expression.right, context, private_names)?;
                if expression.operator == BinaryOperator::In
                    && matches!(
                        expression.left,
                        Expression::Binary(ref nested)
                            if nested.operator == BinaryOperator::In
                                && matches!(nested.left, Expression::PrivateIdentifier(_))
                    )
                {
                    return Err(ParseError::new(
                        "private identifier 'in' expressions cannot be nested",
                        expression.span,
                    ));
                }
                if expression.operator == BinaryOperator::Exponentiate
                    && matches!(expression.left, Expression::Unary(_))
                    && !self.is_parenthesized_expression(&expression.left)
                {
                    return Err(ParseError::new(
                        "unary expressions cannot appear on the left-hand side of '**'",
                        expression.left.span(),
                    ));
                }
                Ok(())
            }
            Expression::Logical(expression) => {
                self.validate_expression(&expression.left, context, private_names)?;
                self.validate_expression(&expression.right, context, private_names)?;
                if matches!(expression.operator, LogicalOperator::NullishCoalescing)
                    && ((!self.is_parenthesized_expression(&expression.left)
                        && self.is_logical_and_or_expression(&expression.left))
                        || (!self.is_parenthesized_expression(&expression.right)
                            && self.is_logical_and_or_expression(&expression.right)))
                {
                    return Err(ParseError::new(
                        "nullish coalescing cannot be mixed with && or || without parentheses",
                        expression.span,
                    ));
                }
                if matches!(
                    expression.operator,
                    LogicalOperator::And | LogicalOperator::Or
                ) && ((!self.is_parenthesized_expression(&expression.left)
                    && self.is_nullish_coalescing_expression(&expression.left))
                    || (!self.is_parenthesized_expression(&expression.right)
                        && self.is_nullish_coalescing_expression(&expression.right)))
                {
                    return Err(ParseError::new(
                        "nullish coalescing cannot be mixed with && or || without parentheses",
                        expression.span,
                    ));
                }
                Ok(())
            }
            Expression::Assignment(expression) => {
                if matches!(
                    expression.operator,
                    AssignmentOperator::LogicalAndAssign
                        | AssignmentOperator::LogicalOrAssign
                        | AssignmentOperator::NullishAssign
                ) && !self.is_valid_logical_assignment_target(&expression.left)
                {
                    return Err(ParseError::new(
                        "invalid assignment target",
                        expression.left.span(),
                    ));
                }
                if !self.is_valid_assignment_target(
                    &expression.left,
                    expression.operator == AssignmentOperator::Assign,
                    context.strict,
                ) {
                    return Err(ParseError::new(
                        "invalid assignment target",
                        expression.left.span(),
                    ));
                }
                if expression.operator == AssignmentOperator::Assign
                    && self.is_parenthesized_expression(&expression.left)
                    && matches!(
                        expression.left,
                        Expression::Array(_) | Expression::Object(_)
                    )
                {
                    return Err(ParseError::new(
                        "invalid assignment target",
                        expression.left.span(),
                    ));
                }
                self.validate_assignment_target_expression(
                    &expression.left,
                    context,
                    private_names,
                )?;
                self.validate_expression(&expression.right, context, private_names)
            }
            Expression::Conditional(expression) => {
                self.validate_expression(&expression.test, context, private_names)?;
                self.validate_expression(&expression.consequent, context, private_names)?;
                self.validate_expression(&expression.alternate, context, private_names)
            }
            Expression::Sequence(expression) => {
                for item in &expression.expressions {
                    self.validate_expression(item, context, private_names)?;
                }
                Ok(())
            }
            Expression::Call(expression) => {
                if let Some(kind) = self.import_callee_kind(&expression.callee) {
                    return self.validate_import_call(expression, context, private_names, kind);
                }
                if matches!(expression.callee, Expression::Super(_)) && !context.super_call_allowed
                {
                    return Err(ParseError::new(
                        "super() is not allowed here",
                        expression.span,
                    ));
                }
                self.validate_expression(&expression.callee, context, private_names)?;
                for argument in &expression.arguments {
                    match argument {
                        CallArgument::Expression(expression) => {
                            self.validate_expression(expression, context, private_names)?
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.validate_expression(argument, context, private_names)?
                        }
                    }
                }
                Ok(())
            }
            Expression::Member(expression) => {
                if matches!(&expression.object, Expression::Identifier(identifier) if identifier.name == "import")
                {
                    return self.validate_import_member(expression, context);
                }
                if matches!(expression.object, Expression::Super(_))
                    && !context.super_property_allowed
                {
                    return Err(ParseError::new(
                        "super property access is not allowed here",
                        expression.span,
                    ));
                }
                self.validate_expression(&expression.object, context, private_names)?;
                if matches!(&expression.object, Expression::Super(_))
                    && matches!(expression.property, MemberProperty::PrivateName(_))
                {
                    return Err(ParseError::new(
                        "private names cannot be accessed on super",
                        expression.span,
                    ));
                }
                if let MemberProperty::Computed { expression, .. } = &expression.property {
                    self.validate_expression(expression, context, private_names)?;
                } else if let MemberProperty::PrivateName(identifier) = &expression.property {
                    self.validate_private_name(identifier, private_names)?;
                }
                Ok(())
            }
            Expression::New(expression) => {
                if self.import_callee_kind(&expression.callee).is_some() {
                    return Err(ParseError::new(
                        "import calls are not constructors",
                        expression.span,
                    ));
                }
                self.validate_expression(&expression.callee, context, private_names)?;
                for argument in &expression.arguments {
                    match argument {
                        CallArgument::Expression(expression) => {
                            self.validate_expression(expression, context, private_names)?
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.validate_expression(argument, context, private_names)?
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn validate_assignment_target_expression(
        &self,
        expression: &Expression,
        context: ValidationContext,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        match expression {
            Expression::Identifier(_) | Expression::Member(_) | Expression::Call(_) => {
                if !self.is_valid_simple_target(expression, context.strict) {
                    return Err(ParseError::new(
                        "invalid assignment target",
                        expression.span(),
                    ));
                }
                self.validate_expression(expression, context, private_names)?;
                self.validate_simple_target_identifier(expression, context)
            }
            Expression::Array(expression) => {
                for element in expression.elements.iter().flatten() {
                    match element {
                        ArrayElement::Expression(expression) => self
                            .validate_assignment_target_expression(
                                expression,
                                context,
                                private_names,
                            )?,
                        ArrayElement::Spread { argument, .. } => self
                            .validate_assignment_target_expression(
                                argument,
                                context,
                                private_names,
                            )?,
                    }
                }
                Ok(())
            }
            Expression::Object(expression) => {
                for property in &expression.properties {
                    match property {
                        ObjectProperty::Property { key, value, .. } => {
                            self.validate_property_key(key, context, private_names)?;
                            self.validate_assignment_target_expression(
                                value,
                                context,
                                private_names,
                            )?;
                        }
                        ObjectProperty::Spread { argument, .. } => self
                            .validate_assignment_target_expression(
                                argument,
                                context,
                                private_names,
                            )?,
                    }
                }
                Ok(())
            }
            Expression::Assignment(expression)
                if expression.operator == AssignmentOperator::Assign =>
            {
                self.validate_assignment_target_expression(
                    &expression.left,
                    context,
                    private_names,
                )?;
                self.validate_expression(&expression.right, context, private_names)
            }
            _ => {
                self.validate_expression(expression, context, private_names)?;
                Err(ParseError::new(
                    "invalid assignment target",
                    expression.span(),
                ))
            }
        }
    }

    fn validate_simple_target_identifier(
        &self,
        expression: &Expression,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        let Expression::Identifier(identifier) = expression else {
            return Ok(());
        };
        if context.strict && matches!(identifier.name.as_str(), "eval" | "arguments" | "yield") {
            return Err(ParseError::new(
                format!(
                    "identifier '{}' is not assignable in strict mode",
                    identifier.name
                ),
                identifier.span,
            ));
        }
        if self.is_always_reserved_identifier_name(&identifier.name)
            && identifier.name != "yield"
            && identifier.name != "await"
        {
            return Err(ParseError::new(
                format!(
                    "identifier '{}' cannot be used as an assignment target",
                    identifier.name
                ),
                identifier.span,
            ));
        }
        Ok(())
    }

    fn validate_identifier_reference(
        &self,
        identifier: &Identifier,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        let name = self
            .decoded_identifier_name(identifier)
            .unwrap_or_else(|| identifier.name.clone());
        if self.identifier_was_escaped(identifier)
            && self.escaped_identifier_is_reserved_here(
                &name,
                context.strict,
                context.await_reserved,
                context.yield_reserved,
            )
        {
            return Err(ParseError::new(
                format!("escaped reserved word '{}' is not allowed here", name),
                identifier.span,
            ));
        }
        if self.is_always_reserved_identifier_name(&name) {
            return Err(ParseError::new(
                format!("identifier '{}' is reserved here", name),
                identifier.span,
            ));
        }
        if context.in_class_static_block && matches!(name.as_str(), "await" | "arguments") {
            return Err(ParseError::new(
                format!("identifier '{}' is reserved here", name),
                identifier.span,
            ));
        }
        if name == "import" {
            return Err(ParseError::new(
                "'import' can only be used as a special form",
                identifier.span,
            ));
        }
        if context.await_reserved && name == "await" {
            return Err(ParseError::new(
                "identifier 'await' is reserved here",
                identifier.span,
            ));
        }
        if context.yield_reserved && name == "yield" {
            return Err(ParseError::new(
                "identifier 'yield' is reserved here",
                identifier.span,
            ));
        }
        if context.strict && self.is_strict_reserved_word(&name) {
            return Err(ParseError::new(
                format!("identifier '{}' is reserved in strict mode", name),
                identifier.span,
            ));
        }
        Ok(())
    }

    fn validate_private_name(
        &self,
        identifier: &Identifier,
        private_names: &HashSet<String>,
    ) -> Result<(), ParseError> {
        if private_names.contains(&identifier.name) {
            Ok(())
        } else {
            Err(ParseError::new(
                format!("private name '#{}' is not defined", identifier.name),
                identifier.span,
            ))
        }
    }

    fn validate_import_call(
        &self,
        expression: &CallExpression,
        context: ValidationContext,
        private_names: &HashSet<String>,
        kind: ImportCallKind,
    ) -> Result<(), ParseError> {
        if expression.optional {
            return Err(ParseError::new(
                "import calls cannot use optional chaining",
                expression.span,
            ));
        }

        let valid_arity = match kind {
            ImportCallKind::Dynamic => (1..=2).contains(&expression.arguments.len()),
            ImportCallKind::Source | ImportCallKind::Defer => expression.arguments.len() == 1,
        };
        if !valid_arity {
            return Err(ParseError::new(
                "invalid import call arity",
                expression.span,
            ));
        }

        for argument in &expression.arguments {
            match argument {
                CallArgument::Expression(expression) => {
                    self.validate_expression(expression, context, private_names)?
                }
                CallArgument::Spread { span, .. } => {
                    return Err(ParseError::new(
                        "import calls do not allow spread arguments",
                        *span,
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_import_member(
        &self,
        expression: &MemberExpression,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        if expression.optional {
            return Err(ParseError::new(
                "import cannot be used with optional chaining",
                expression.span,
            ));
        }

        match &expression.property {
            MemberProperty::Identifier(identifier) if identifier.name == "meta" => {
                if self.identifier_was_escaped(identifier) {
                    return Err(ParseError::new(
                        "escaped 'meta' is not allowed in import.meta",
                        identifier.span,
                    ));
                }
                if !context.module_await_reserved {
                    return Err(ParseError::new(
                        "import.meta is only allowed in module code",
                        expression.span,
                    ));
                }
                Ok(())
            }
            MemberProperty::Identifier(identifier)
                if matches!(identifier.name.as_str(), "source" | "defer") =>
            {
                Err(ParseError::new(
                    format!("import.{} must be called", identifier.name),
                    expression.span,
                ))
            }
            _ => Err(ParseError::new("invalid use of import", expression.span)),
        }
    }

    fn property_key_name<'a>(&self, key: &'a PropertyKey) -> Option<&'a str> {
        match key {
            PropertyKey::Identifier(identifier) => Some(&identifier.name),
            PropertyKey::String(literal) => Some(&literal.value),
            _ => None,
        }
    }

    fn token_index_at_offset(&self, offset: usize) -> Option<usize> {
        self.tokens
            .iter()
            .position(|token| token.span.start.offset == offset)
    }

    fn source_slice(&self, span: Span) -> Option<&str> {
        self.source.get(span.start.offset..span.end.offset)
    }

    fn validate_template_literal(
        &self,
        template: &TemplateLiteral,
        context: ValidationContext,
    ) -> Result<(), ParseError> {
        if context.strict
            && self
                .source_slice(template.span)
                .is_some_and(|raw| self.template_substitutions_have_legacy_escape(raw))
        {
            return Err(ParseError::new(
                "legacy string escapes are not allowed in strict mode",
                template.span,
            ));
        }
        Ok(())
    }

    fn decoded_identifier_name(&self, identifier: &Identifier) -> Option<String> {
        let raw = self.source_slice(identifier.span)?;
        if !raw.contains('\\') {
            return Some(raw.to_string());
        }
        let mut decoded = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                decoded.push(ch);
                continue;
            }
            if chars.next() != Some('u') {
                return None;
            }
            let value = if chars.peek() == Some(&'{') {
                chars.next();
                let mut hex = String::new();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '}' {
                        break;
                    }
                    hex.push(next);
                }
                u32::from_str_radix(&hex, 16).ok()?
            } else {
                let mut hex = String::new();
                for _ in 0..4 {
                    hex.push(chars.next()?);
                }
                u32::from_str_radix(&hex, 16).ok()?
            };
            decoded.push(char::from_u32(value)?);
        }
        Some(decoded)
    }

    fn expression_starts_with_linebreak_let_bracket(&self, expression: &Expression) -> bool {
        let Some(index) = self.token_index_at_offset(expression.span().start.offset) else {
            return false;
        };
        self.tokens.get(index).map(Token::tag) == Some(TokenTag::Let)
            && self.tokens.get(index + 1).is_some_and(|token| {
                token.leading_line_break && token.tag() == TokenTag::LeftBracket
            })
    }

    fn is_proto_data_property_name(&self, key: &PropertyKey) -> bool {
        matches!(self.property_key_name(key), Some("__proto__"))
    }

    fn is_strict_legacy_decimal_literal(&self, raw: &str) -> bool {
        raw.len() > 1 && raw.starts_with('0') && raw.chars().skip(1).all(|ch| ch.is_ascii_digit())
    }

    fn numeric_literal_followed_by_identifier(&self, span: Span) -> bool {
        let Some(rest) = self.source.get(span.end.offset..) else {
            return false;
        };
        rest.chars().next().is_some_and(|ch| {
            ch == '$' || ch == '_' || ch == '\\' || ch.is_ascii_alphabetic() || ch.is_ascii_digit()
        })
    }

    fn raw_string_has_legacy_escape(&self, raw: &str) -> bool {
        let mut chars = raw.chars().peekable();
        let mut escaped = false;
        while let Some(ch) = chars.next() {
            if !escaped {
                escaped = ch == '\\';
                continue;
            }
            escaped = false;
            match ch {
                '0' => {
                    if chars.peek().is_some_and(|next| next.is_ascii_digit()) {
                        return true;
                    }
                }
                '1'..='9' => return true,
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                }
                '\n' | '\u{2028}' | '\u{2029}' => {}
                _ => {}
            }
        }
        false
    }

    fn raw_string_is_well_formed_unicode(&self, raw: &str) -> bool {
        let mut chars = raw.chars();
        let Some(quote) = chars.next() else {
            return true;
        };
        if quote != '\'' && quote != '"' {
            return true;
        }

        let mut pending_high_surrogate = false;
        while let Some(ch) = chars.next() {
            if ch == quote {
                return !pending_high_surrogate;
            }

            if ch != '\\' {
                if pending_high_surrogate {
                    return false;
                }
                continue;
            }

            let Some(escape) = chars.next() else {
                return false;
            };

            let code_point = match escape {
                'u' => {
                    let Some(value) = self.read_raw_unicode_escape(&mut chars) else {
                        return false;
                    };
                    Some(value)
                }
                '\r' => {
                    if chars.clone().next() == Some('\n') {
                        chars.next();
                    }
                    None
                }
                '\n' | '\u{2028}' | '\u{2029}' => None,
                _ => None,
            };

            match code_point {
                Some(0xd800..=0xdbff) => {
                    if pending_high_surrogate {
                        return false;
                    }
                    pending_high_surrogate = true;
                }
                Some(0xdc00..=0xdfff) => {
                    if pending_high_surrogate {
                        pending_high_surrogate = false;
                    } else {
                        return false;
                    }
                }
                Some(_) | None => {
                    if pending_high_surrogate {
                        return false;
                    }
                }
            }
        }

        !pending_high_surrogate
    }

    fn read_raw_unicode_escape(&self, chars: &mut std::str::Chars<'_>) -> Option<u32> {
        let rest = chars.as_str();
        if let Some(stripped) = rest.strip_prefix('{') {
            let end = stripped.find('}')?;
            let digits = &stripped[..end];
            if digits.is_empty() {
                return None;
            }
            let value = u32::from_str_radix(digits, 16).ok()?;
            let consumed = 1 + end + 1;
            *chars = rest[consumed..].chars();
            Some(value)
        } else {
            let digits = rest.get(..4)?;
            if !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return None;
            }
            let value = u32::from_str_radix(digits, 16).ok()?;
            *chars = rest[4..].chars();
            Some(value)
        }
    }

    fn template_substitutions_have_legacy_escape(&self, raw: &str) -> bool {
        let mut index = 0usize;
        if self.char_at(raw, index) != Some('`') {
            return false;
        }
        index += '`'.len_utf8();

        while let Some(ch) = self.char_at(raw, index) {
            match ch {
                '`' => return false,
                '\\' => {
                    index = self.skip_template_escape(raw, index);
                }
                '$' if self.char_at(raw, index + 1) == Some('{') => {
                    index += 2;
                    if self.template_expression_has_legacy_escape(raw, &mut index, 1) {
                        return true;
                    }
                }
                _ => index += ch.len_utf8(),
            }
        }

        false
    }

    fn template_expression_has_legacy_escape(
        &self,
        raw: &str,
        index: &mut usize,
        mut depth: usize,
    ) -> bool {
        while depth > 0 {
            let Some(ch) = self.char_at(raw, *index) else {
                return false;
            };

            match ch {
                '\'' | '"' => {
                    let start = *index;
                    if !self.skip_string_literal(raw, index, ch) {
                        return false;
                    }
                    if self.raw_string_has_legacy_escape(&raw[start..*index]) {
                        return true;
                    }
                }
                '`' => {
                    if self.nested_template_has_legacy_escape(raw, index) {
                        return true;
                    }
                }
                '/' if raw[*index..].starts_with("//") => {
                    *index += 2;
                    while let Some(next) = self.char_at(raw, *index) {
                        if is_line_terminator(next) {
                            break;
                        }
                        *index += next.len_utf8();
                    }
                }
                '/' if raw[*index..].starts_with("/*") => {
                    *index += 2;
                    while *index < raw.len() && !raw[*index..].starts_with("*/") {
                        let Some(next) = self.char_at(raw, *index) else {
                            return false;
                        };
                        *index += next.len_utf8();
                    }
                    if *index >= raw.len() {
                        return false;
                    }
                    *index += 2;
                }
                '{' => {
                    depth += 1;
                    *index += 1;
                }
                '}' => {
                    depth -= 1;
                    *index += 1;
                }
                _ => *index += ch.len_utf8(),
            }
        }

        false
    }

    fn nested_template_has_legacy_escape(&self, raw: &str, index: &mut usize) -> bool {
        *index += 1;
        while let Some(ch) = self.char_at(raw, *index) {
            match ch {
                '`' => {
                    *index += 1;
                    return false;
                }
                '\\' => {
                    *index = self.skip_template_escape(raw, *index);
                }
                '$' if self.char_at(raw, *index + 1) == Some('{') => {
                    *index += 2;
                    if self.template_expression_has_legacy_escape(raw, index, 1) {
                        return true;
                    }
                }
                _ => *index += ch.len_utf8(),
            }
        }

        false
    }

    fn skip_string_literal(&self, raw: &str, index: &mut usize, quote: char) -> bool {
        *index += quote.len_utf8();
        while let Some(ch) = self.char_at(raw, *index) {
            match ch {
                c if c == quote => {
                    *index += c.len_utf8();
                    return true;
                }
                '\\' => {
                    *index += 1;
                    let Some(next) = self.char_at(raw, *index) else {
                        return false;
                    };
                    if is_line_terminator(next) {
                        *index = self.skip_line_terminator(raw, *index);
                    } else {
                        *index += next.len_utf8();
                    }
                }
                c if is_line_terminator(c) => return false,
                _ => *index += ch.len_utf8(),
            }
        }

        false
    }

    fn skip_template_escape(&self, raw: &str, index: usize) -> usize {
        let next = index + 1;
        match self.char_at(raw, next) {
            Some(ch) if is_line_terminator(ch) => self.skip_line_terminator(raw, next),
            Some(ch) => next + ch.len_utf8(),
            None => next,
        }
    }

    fn skip_line_terminator(&self, raw: &str, index: usize) -> usize {
        match self.char_at(raw, index) {
            Some('\r') if self.char_at(raw, index + 1) == Some('\n') => index + 2,
            Some(ch) if is_line_terminator(ch) => index + ch.len_utf8(),
            _ => index,
        }
    }

    fn char_at(&self, raw: &str, index: usize) -> Option<char> {
        raw.get(index..)?.chars().next()
    }

    fn expression_contains_optional_chain(expression: &Expression) -> bool {
        match expression {
            Expression::Call(call) => {
                call.optional
                    || Self::expression_contains_optional_chain(&call.callee)
                    || call.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            Self::expression_contains_optional_chain(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            Self::expression_contains_optional_chain(argument)
                        }
                    })
            }
            Expression::Member(member) => {
                member.optional
                    || Self::expression_contains_optional_chain(&member.object)
                    || matches!(
                        &member.property,
                        MemberProperty::Computed { expression, .. }
                            if Self::expression_contains_optional_chain(expression)
                    )
            }
            _ => false,
        }
    }

    fn property_key_private_name<'a>(&self, key: &'a PropertyKey) -> Option<&'a Identifier> {
        match key {
            PropertyKey::PrivateName(identifier) => Some(identifier),
            _ => None,
        }
    }

    fn collect_class_private_names(&self, class: &Class) -> Result<HashSet<String>, ParseError> {
        let mut constructor_seen = false;
        let mut private_names = HashSet::new();
        let mut declarations: HashMap<String, Vec<PrivateNameDeclaration>> = HashMap::new();

        for element in &class.body {
            match element {
                ClassElement::Empty(_) | ClassElement::StaticBlock(_) => {}
                ClassElement::Method(method) => {
                    self.validate_class_method_name_rules(method)?;
                    if method.kind == MethodKind::Constructor {
                        if constructor_seen {
                            return Err(ParseError::new(
                                "duplicate constructor in class body",
                                method.span,
                            ));
                        }
                        constructor_seen = true;
                    }
                    if let Some(identifier) = self.property_key_private_name(&method.key) {
                        private_names.insert(identifier.name.clone());
                        declarations
                            .entry(identifier.name.clone())
                            .or_default()
                            .push(PrivateNameDeclaration {
                                is_static: method.is_static,
                                kind: match method.kind {
                                    MethodKind::Getter => PrivateNameDeclarationKind::Getter,
                                    MethodKind::Setter => PrivateNameDeclarationKind::Setter,
                                    MethodKind::Method | MethodKind::Constructor => {
                                        PrivateNameDeclarationKind::Other
                                    }
                                },
                                span: identifier.span,
                            });
                    }
                }
                ClassElement::Field(field) => {
                    self.validate_class_field_name_rules(field)?;
                    if let Some(identifier) = self.property_key_private_name(&field.key) {
                        private_names.insert(identifier.name.clone());
                        declarations
                            .entry(identifier.name.clone())
                            .or_default()
                            .push(PrivateNameDeclaration {
                                is_static: field.is_static,
                                kind: PrivateNameDeclarationKind::Other,
                                span: identifier.span,
                            });
                    }
                }
            }
        }

        for declaration in declarations.values() {
            if let Some(span) = self.invalid_private_name_declaration_span(declaration) {
                return Err(ParseError::new(
                    "duplicate private name in class body",
                    span,
                ));
            }
        }

        Ok(private_names)
    }

    fn validate_class_method_name_rules(&self, method: &ClassMethod) -> Result<(), ParseError> {
        if method.is_static && self.property_key_name(&method.key) == Some("prototype") {
            return Err(ParseError::new(
                "static class elements cannot be named 'prototype'",
                method.key.span(),
            ));
        }
        if let Some(identifier) = self.property_key_private_name(&method.key)
            && identifier.name == "constructor"
        {
            return Err(ParseError::new(
                "private class elements cannot be named '#constructor'",
                identifier.span,
            ));
        }
        Ok(())
    }

    fn validate_class_field_name_rules(&self, field: &ClassField) -> Result<(), ParseError> {
        if let Some(name) = self.property_key_name(&field.key) {
            if name == "constructor" {
                return Err(ParseError::new(
                    "class fields cannot be named 'constructor'",
                    field.key.span(),
                ));
            }
            if field.is_static && name == "prototype" {
                return Err(ParseError::new(
                    "static class fields cannot be named 'prototype'",
                    field.key.span(),
                ));
            }
        }
        if let Some(identifier) = self.property_key_private_name(&field.key)
            && identifier.name == "constructor"
        {
            return Err(ParseError::new(
                "private class elements cannot be named '#constructor'",
                identifier.span,
            ));
        }
        Ok(())
    }

    fn invalid_private_name_declaration_span(
        &self,
        declarations: &[PrivateNameDeclaration],
    ) -> Option<Span> {
        if declarations.len() <= 1 {
            return None;
        }

        let is_static = declarations[0].is_static;
        if declarations.iter().any(|item| item.is_static != is_static) {
            return Some(declarations[1].span);
        }

        let mut getter_count = 0;
        let mut setter_count = 0;
        let mut other_count = 0;
        for declaration in declarations {
            match declaration.kind {
                PrivateNameDeclarationKind::Getter => getter_count += 1,
                PrivateNameDeclarationKind::Setter => setter_count += 1,
                PrivateNameDeclarationKind::Other => other_count += 1,
            }
        }

        if declarations.len() == 2 && getter_count == 1 && setter_count == 1 && other_count == 0 {
            None
        } else {
            Some(declarations[1].span)
        }
    }

    fn import_callee_kind(&self, expression: &Expression) -> Option<ImportCallKind> {
        match expression {
            Expression::Identifier(identifier) if identifier.name == "import" => {
                Some(ImportCallKind::Dynamic)
            }
            Expression::Member(member) if !member.optional => {
                let Expression::Identifier(identifier) = &member.object else {
                    return None;
                };
                if identifier.name != "import" {
                    return None;
                }
                match &member.property {
                    MemberProperty::Identifier(identifier) if identifier.name == "source" => {
                        Some(ImportCallKind::Source)
                    }
                    MemberProperty::Identifier(identifier) if identifier.name == "defer" => {
                        Some(ImportCallKind::Defer)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_import_meta_expression(&self, expression: &Expression) -> bool {
        matches!(
            expression,
            Expression::Member(member)
                if !member.optional
                    && matches!(&member.object, Expression::Identifier(identifier) if identifier.name == "import")
                    && matches!(&member.property, MemberProperty::Identifier(identifier) if identifier.name == "meta")
        )
    }

    fn is_private_member_expression(&self, expression: &Expression) -> bool {
        matches!(expression, Expression::Member(member) if matches!(member.property, MemberProperty::PrivateName(_)))
    }

    fn statement_list_contains_super_call(&self, statements: &[Statement]) -> bool {
        statements
            .iter()
            .any(|statement| self.statement_contains_super_call(statement))
    }

    fn statement_contains_super_call(&self, statement: &Statement) -> bool {
        match statement {
            Statement::Directive(_) | Statement::Empty(_) | Statement::Debugger(_) => false,
            Statement::Block(block) => self.statement_list_contains_super_call(&block.body),
            Statement::Labeled(statement) => {
                self.statement_contains_super_call(statement.body.as_ref())
            }
            Statement::ImportDeclaration(declaration) => declaration
                .attributes
                .as_ref()
                .is_some_and(|attributes| self.expression_contains_super_call(attributes)),
            Statement::ExportDeclaration(declaration) => match declaration {
                ExportDeclaration::All(declaration) => declaration
                    .attributes
                    .as_ref()
                    .is_some_and(|attributes| self.expression_contains_super_call(attributes)),
                ExportDeclaration::Named(declaration) => declaration
                    .attributes
                    .as_ref()
                    .is_some_and(|attributes| self.expression_contains_super_call(attributes)),
                ExportDeclaration::Default(declaration) => match &declaration.declaration {
                    ExportDefaultKind::Expression(expression) => {
                        self.expression_contains_super_call(expression)
                    }
                    ExportDefaultKind::Function(function) => {
                        self.function_contains_super_call(function)
                    }
                    ExportDefaultKind::Class(_) => false,
                },
                ExportDeclaration::Declaration(declaration) => match declaration {
                    ExportedDeclaration::Variable(declaration) => {
                        declaration.declarations.iter().any(|item| {
                            item.init
                                .as_ref()
                                .is_some_and(|init| self.expression_contains_super_call(init))
                        })
                    }
                    ExportedDeclaration::Function(function) => {
                        self.function_contains_super_call(function)
                    }
                    ExportedDeclaration::Class(_) => false,
                },
            },
            Statement::VariableDeclaration(declaration) => {
                declaration.declarations.iter().any(|item| {
                    item.init
                        .as_ref()
                        .is_some_and(|init| self.expression_contains_super_call(init))
                })
            }
            Statement::FunctionDeclaration(function) => self.function_contains_super_call(function),
            Statement::ClassDeclaration(_) => false,
            Statement::If(statement) => {
                self.expression_contains_super_call(&statement.test)
                    || self.statement_contains_super_call(statement.consequent.as_ref())
                    || statement
                        .alternate
                        .as_ref()
                        .is_some_and(|alternate| self.statement_contains_super_call(alternate))
            }
            Statement::While(statement) => {
                self.expression_contains_super_call(&statement.test)
                    || self.statement_contains_super_call(statement.body.as_ref())
            }
            Statement::DoWhile(statement) => {
                self.statement_contains_super_call(statement.body.as_ref())
                    || self.expression_contains_super_call(&statement.test)
            }
            Statement::For(statement) => match statement {
                ForStatement::Classic(statement) => {
                    statement.init.as_ref().is_some_and(|init| match init {
                        ForInit::VariableDeclaration(declaration) => {
                            declaration.declarations.iter().any(|item| {
                                item.init
                                    .as_ref()
                                    .is_some_and(|init| self.expression_contains_super_call(init))
                            })
                        }
                        ForInit::Expression(expression) => {
                            self.expression_contains_super_call(expression)
                        }
                    }) || statement
                        .test
                        .as_ref()
                        .is_some_and(|test| self.expression_contains_super_call(test))
                        || statement
                            .update
                            .as_ref()
                            .is_some_and(|update| self.expression_contains_super_call(update))
                        || self.statement_contains_super_call(statement.body.as_ref())
                }
                ForStatement::In(statement) | ForStatement::Of(statement) => {
                    (match &statement.left {
                        ForLeft::VariableDeclaration(declaration) => {
                            declaration.declarations.iter().any(|item| {
                                item.init
                                    .as_ref()
                                    .is_some_and(|init| self.expression_contains_super_call(init))
                            })
                        }
                        ForLeft::Pattern(pattern) => self.pattern_contains_super_call(pattern),
                        ForLeft::Expression(expression) => {
                            self.expression_contains_super_call(expression)
                        }
                    }) || self.expression_contains_super_call(&statement.right)
                        || self.statement_contains_super_call(statement.body.as_ref())
                }
            },
            Statement::Switch(statement) => {
                self.expression_contains_super_call(&statement.discriminant)
                    || statement.cases.iter().any(|case| {
                        case.test
                            .as_ref()
                            .is_some_and(|test| self.expression_contains_super_call(test))
                            || self.statement_list_contains_super_call(&case.consequent)
                    })
            }
            Statement::Return(statement) => statement
                .argument
                .as_ref()
                .is_some_and(|argument| self.expression_contains_super_call(argument)),
            Statement::Break(_) | Statement::Continue(_) => false,
            Statement::Throw(statement) => self.expression_contains_super_call(&statement.argument),
            Statement::Try(statement) => {
                self.statement_list_contains_super_call(&statement.block.body)
                    || statement.handler.as_ref().is_some_and(|handler| {
                        handler
                            .param
                            .as_ref()
                            .is_some_and(|param| self.pattern_contains_super_call(param))
                            || self.statement_list_contains_super_call(&handler.body.body)
                    })
                    || statement.finalizer.as_ref().is_some_and(|finalizer| {
                        self.statement_list_contains_super_call(&finalizer.body)
                    })
            }
            Statement::With(statement) => {
                self.expression_contains_super_call(&statement.object)
                    || self.statement_contains_super_call(statement.body.as_ref())
            }
            Statement::Expression(statement) => {
                self.expression_contains_super_call(&statement.expression)
            }
        }
    }

    fn function_contains_super_call(&self, function: &Function) -> bool {
        function
            .params
            .iter()
            .any(|pattern| self.pattern_contains_super_call(pattern))
            || self.statement_list_contains_super_call(&function.body.body)
    }

    fn arrow_function_contains_super_call(&self, function: &ArrowFunction) -> bool {
        function
            .params
            .iter()
            .any(|pattern| self.pattern_contains_super_call(pattern))
            || match &function.body {
                ArrowBody::Expression(expression) => {
                    self.expression_contains_super_call(expression)
                }
                ArrowBody::Block(block) => self.statement_list_contains_super_call(&block.body),
            }
    }

    fn pattern_contains_super_call(&self, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Identifier(_) => false,
            Pattern::Array(pattern) => pattern
                .elements
                .iter()
                .flatten()
                .any(|element| self.pattern_contains_super_call(element)),
            Pattern::Object(pattern) => pattern.properties.iter().any(|property| match property {
                ObjectPatternProperty::Property { value, .. } => {
                    self.pattern_contains_super_call(value)
                }
                ObjectPatternProperty::Rest { argument, .. } => {
                    self.pattern_contains_super_call(argument)
                }
            }),
            Pattern::Rest(pattern) => self.pattern_contains_super_call(pattern.argument.as_ref()),
            Pattern::Assignment(pattern) => {
                self.pattern_contains_super_call(pattern.left.as_ref())
                    || self.expression_contains_super_call(&pattern.right)
            }
        }
    }

    fn expression_contains_super_call(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Identifier(_)
            | Expression::PrivateIdentifier(_)
            | Expression::Literal(_)
            | Expression::This(_)
            | Expression::Super(_)
            | Expression::MetaProperty(_)
            | Expression::Class(_) => false,
            Expression::Array(expression) => expression.elements.iter().flatten().any(|element| match element {
                ArrayElement::Expression(expression) => self.expression_contains_super_call(expression),
                ArrayElement::Spread { argument, .. } => self.expression_contains_super_call(argument),
            }),
            Expression::Object(expression) => expression.properties.iter().any(|property| match property {
                ObjectProperty::Property { key, value, .. } => {
                    matches!(key, PropertyKey::Computed { expression, .. } if self.expression_contains_super_call(expression))
                        || self.expression_contains_super_call(value)
                }
                ObjectProperty::Spread { argument, .. } => self.expression_contains_super_call(argument),
            }),
            Expression::Function(function) => self.function_contains_super_call(function),
            Expression::ArrowFunction(function) => self.arrow_function_contains_super_call(function),
            Expression::TaggedTemplate(expression) => self.expression_contains_super_call(&expression.tag),
            Expression::Yield(expression) => expression
                .argument
                .as_ref()
                .is_some_and(|argument| self.expression_contains_super_call(argument)),
            Expression::Await(expression) => self.expression_contains_super_call(&expression.argument),
            Expression::Unary(expression) => self.expression_contains_super_call(&expression.argument),
            Expression::Update(expression) => self.expression_contains_super_call(&expression.argument),
            Expression::Binary(expression) => {
                self.expression_contains_super_call(&expression.left)
                    || self.expression_contains_super_call(&expression.right)
            }
            Expression::Logical(expression) => {
                self.expression_contains_super_call(&expression.left)
                    || self.expression_contains_super_call(&expression.right)
            }
            Expression::Assignment(expression) => {
                self.expression_contains_super_call(&expression.left)
                    || self.expression_contains_super_call(&expression.right)
            }
            Expression::Conditional(expression) => {
                self.expression_contains_super_call(&expression.test)
                    || self.expression_contains_super_call(&expression.consequent)
                    || self.expression_contains_super_call(&expression.alternate)
            }
            Expression::Sequence(expression) => expression
                .expressions
                .iter()
                .any(|item| self.expression_contains_super_call(item)),
            Expression::Call(expression) => {
                matches!(expression.callee, Expression::Super(_))
                    || self.expression_contains_super_call(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            self.expression_contains_super_call(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.expression_contains_super_call(argument)
                        }
                    })
            }
            Expression::Member(expression) => {
                self.expression_contains_super_call(&expression.object)
                    || matches!(
                        &expression.property,
                        MemberProperty::Computed { expression, .. }
                            if self.expression_contains_super_call(expression)
                    )
            }
            Expression::New(expression) => {
                self.expression_contains_super_call(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            self.expression_contains_super_call(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.expression_contains_super_call(argument)
                        }
                    })
            }
        }
    }

    fn expression_contains_arguments(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Identifier(identifier) => identifier.name == "arguments",
            Expression::PrivateIdentifier(_)
            | Expression::Literal(_)
            | Expression::This(_)
            | Expression::Super(_)
            | Expression::MetaProperty(_)
            | Expression::Class(_) => false,
            Expression::Array(expression) => expression.elements.iter().flatten().any(|element| match element {
                ArrayElement::Expression(expression) => self.expression_contains_arguments(expression),
                ArrayElement::Spread { argument, .. } => self.expression_contains_arguments(argument),
            }),
            Expression::Object(expression) => expression.properties.iter().any(|property| match property {
                ObjectProperty::Property { key, value, .. } => {
                    matches!(key, PropertyKey::Computed { expression, .. } if self.expression_contains_arguments(expression))
                        || self.expression_contains_arguments(value)
                }
                ObjectProperty::Spread { argument, .. } => self.expression_contains_arguments(argument),
            }),
            Expression::Function(function) => self.function_contains_arguments(function),
            Expression::ArrowFunction(function) => self.arrow_function_contains_arguments(function),
            Expression::TaggedTemplate(expression) => self.expression_contains_arguments(&expression.tag),
            Expression::Yield(expression) => expression
                .argument
                .as_ref()
                .is_some_and(|argument| self.expression_contains_arguments(argument)),
            Expression::Await(expression) => self.expression_contains_arguments(&expression.argument),
            Expression::Unary(expression) => self.expression_contains_arguments(&expression.argument),
            Expression::Update(expression) => self.expression_contains_arguments(&expression.argument),
            Expression::Binary(expression) => {
                self.expression_contains_arguments(&expression.left)
                    || self.expression_contains_arguments(&expression.right)
            }
            Expression::Logical(expression) => {
                self.expression_contains_arguments(&expression.left)
                    || self.expression_contains_arguments(&expression.right)
            }
            Expression::Assignment(expression) => {
                self.expression_contains_arguments(&expression.left)
                    || self.expression_contains_arguments(&expression.right)
            }
            Expression::Conditional(expression) => {
                self.expression_contains_arguments(&expression.test)
                    || self.expression_contains_arguments(&expression.consequent)
                    || self.expression_contains_arguments(&expression.alternate)
            }
            Expression::Sequence(expression) => expression
                .expressions
                .iter()
                .any(|item| self.expression_contains_arguments(item)),
            Expression::Call(expression) => {
                self.expression_contains_arguments(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            self.expression_contains_arguments(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.expression_contains_arguments(argument)
                        }
                    })
            }
            Expression::Member(expression) => {
                self.expression_contains_arguments(&expression.object)
                    || matches!(
                        &expression.property,
                        MemberProperty::Computed { expression, .. }
                            if self.expression_contains_arguments(expression)
                    )
            }
            Expression::New(expression) => {
                self.expression_contains_arguments(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            self.expression_contains_arguments(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.expression_contains_arguments(argument)
                        }
                    })
            }
        }
    }

    fn function_contains_arguments(&self, function: &Function) -> bool {
        function
            .params
            .iter()
            .any(|pattern| self.pattern_contains_arguments(pattern))
            || self.statement_list_contains_arguments(&function.body.body)
    }

    fn arrow_function_contains_arguments(&self, function: &ArrowFunction) -> bool {
        function
            .params
            .iter()
            .any(|pattern| self.pattern_contains_arguments(pattern))
            || match &function.body {
                ArrowBody::Expression(expression) => self.expression_contains_arguments(expression),
                ArrowBody::Block(block) => self.statement_list_contains_arguments(&block.body),
            }
    }

    fn statement_list_contains_arguments(&self, statements: &[Statement]) -> bool {
        statements
            .iter()
            .any(|statement| self.statement_contains_arguments(statement))
    }

    fn statement_contains_arguments(&self, statement: &Statement) -> bool {
        match statement {
            Statement::Directive(_) | Statement::Empty(_) | Statement::Debugger(_) => false,
            Statement::Block(block) => self.statement_list_contains_arguments(&block.body),
            Statement::Labeled(statement) => {
                self.statement_contains_arguments(statement.body.as_ref())
            }
            Statement::ImportDeclaration(declaration) => declaration
                .attributes
                .as_ref()
                .is_some_and(|attributes| self.expression_contains_arguments(attributes)),
            Statement::ExportDeclaration(declaration) => match declaration {
                ExportDeclaration::All(declaration) => declaration
                    .attributes
                    .as_ref()
                    .is_some_and(|attributes| self.expression_contains_arguments(attributes)),
                ExportDeclaration::Named(declaration) => declaration
                    .attributes
                    .as_ref()
                    .is_some_and(|attributes| self.expression_contains_arguments(attributes)),
                ExportDeclaration::Default(declaration) => match &declaration.declaration {
                    ExportDefaultKind::Expression(expression) => {
                        self.expression_contains_arguments(expression)
                    }
                    ExportDefaultKind::Function(function) => {
                        self.function_contains_arguments(function)
                    }
                    ExportDefaultKind::Class(_) => false,
                },
                ExportDeclaration::Declaration(declaration) => match declaration {
                    ExportedDeclaration::Variable(declaration) => {
                        declaration.declarations.iter().any(|item| {
                            item.init
                                .as_ref()
                                .is_some_and(|init| self.expression_contains_arguments(init))
                        })
                    }
                    ExportedDeclaration::Function(function) => {
                        self.function_contains_arguments(function)
                    }
                    ExportedDeclaration::Class(_) => false,
                },
            },
            Statement::VariableDeclaration(declaration) => {
                declaration.declarations.iter().any(|item| {
                    item.init
                        .as_ref()
                        .is_some_and(|init| self.expression_contains_arguments(init))
                })
            }
            Statement::FunctionDeclaration(function) => self.function_contains_arguments(function),
            Statement::ClassDeclaration(_) => false,
            Statement::If(statement) => {
                self.expression_contains_arguments(&statement.test)
                    || self.statement_contains_arguments(statement.consequent.as_ref())
                    || statement
                        .alternate
                        .as_ref()
                        .is_some_and(|alternate| self.statement_contains_arguments(alternate))
            }
            Statement::While(statement) => {
                self.expression_contains_arguments(&statement.test)
                    || self.statement_contains_arguments(statement.body.as_ref())
            }
            Statement::DoWhile(statement) => {
                self.statement_contains_arguments(statement.body.as_ref())
                    || self.expression_contains_arguments(&statement.test)
            }
            Statement::For(statement) => match statement {
                ForStatement::Classic(statement) => {
                    statement.init.as_ref().is_some_and(|init| match init {
                        ForInit::VariableDeclaration(declaration) => {
                            declaration.declarations.iter().any(|item| {
                                item.init
                                    .as_ref()
                                    .is_some_and(|init| self.expression_contains_arguments(init))
                            })
                        }
                        ForInit::Expression(expression) => {
                            self.expression_contains_arguments(expression)
                        }
                    }) || statement
                        .test
                        .as_ref()
                        .is_some_and(|test| self.expression_contains_arguments(test))
                        || statement
                            .update
                            .as_ref()
                            .is_some_and(|update| self.expression_contains_arguments(update))
                        || self.statement_contains_arguments(statement.body.as_ref())
                }
                ForStatement::In(statement) | ForStatement::Of(statement) => {
                    (match &statement.left {
                        ForLeft::VariableDeclaration(declaration) => {
                            declaration.declarations.iter().any(|item| {
                                item.init
                                    .as_ref()
                                    .is_some_and(|init| self.expression_contains_arguments(init))
                            })
                        }
                        ForLeft::Pattern(pattern) => self.pattern_contains_arguments(pattern),
                        ForLeft::Expression(expression) => {
                            self.expression_contains_arguments(expression)
                        }
                    }) || self.expression_contains_arguments(&statement.right)
                        || self.statement_contains_arguments(statement.body.as_ref())
                }
            },
            Statement::Switch(statement) => {
                self.expression_contains_arguments(&statement.discriminant)
                    || statement.cases.iter().any(|case| {
                        case.test
                            .as_ref()
                            .is_some_and(|test| self.expression_contains_arguments(test))
                            || self.statement_list_contains_arguments(&case.consequent)
                    })
            }
            Statement::Return(statement) => statement
                .argument
                .as_ref()
                .is_some_and(|argument| self.expression_contains_arguments(argument)),
            Statement::Break(_) | Statement::Continue(_) => false,
            Statement::Throw(statement) => self.expression_contains_arguments(&statement.argument),
            Statement::Try(statement) => {
                self.statement_list_contains_arguments(&statement.block.body)
                    || statement.handler.as_ref().is_some_and(|handler| {
                        handler
                            .param
                            .as_ref()
                            .is_some_and(|param| self.pattern_contains_arguments(param))
                            || self.statement_list_contains_arguments(&handler.body.body)
                    })
                    || statement.finalizer.as_ref().is_some_and(|finalizer| {
                        self.statement_list_contains_arguments(&finalizer.body)
                    })
            }
            Statement::With(statement) => {
                self.expression_contains_arguments(&statement.object)
                    || self.statement_contains_arguments(statement.body.as_ref())
            }
            Statement::Expression(statement) => {
                self.expression_contains_arguments(&statement.expression)
            }
        }
    }

    fn pattern_contains_arguments(&self, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Identifier(_) => false,
            Pattern::Array(pattern) => pattern
                .elements
                .iter()
                .flatten()
                .any(|element| self.pattern_contains_arguments(element)),
            Pattern::Object(pattern) => pattern.properties.iter().any(|property| match property {
                ObjectPatternProperty::Property { value, .. } => {
                    self.pattern_contains_arguments(value)
                }
                ObjectPatternProperty::Rest { argument, .. } => {
                    self.pattern_contains_arguments(argument)
                }
            }),
            Pattern::Rest(pattern) => self.pattern_contains_arguments(pattern.argument.as_ref()),
            Pattern::Assignment(pattern) => {
                self.pattern_contains_arguments(pattern.left.as_ref())
                    || self.expression_contains_arguments(&pattern.right)
            }
        }
    }

    fn patterns_contain_await_expression(&self, patterns: &[Pattern]) -> bool {
        patterns.iter().any(Self::pattern_contains_await_expression)
    }

    fn pattern_contains_await_expression(pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Identifier(_) => false,
            Pattern::Array(pattern) => pattern
                .elements
                .iter()
                .flatten()
                .any(Self::pattern_contains_await_expression),
            Pattern::Object(pattern) => pattern.properties.iter().any(|property| match property {
                ObjectPatternProperty::Property { value, .. } => {
                    Self::pattern_contains_await_expression(value)
                }
                ObjectPatternProperty::Rest { argument, .. } => {
                    Self::pattern_contains_await_expression(argument)
                }
            }),
            Pattern::Rest(pattern) => {
                Self::pattern_contains_await_expression(pattern.argument.as_ref())
            }
            Pattern::Assignment(pattern) => {
                Self::pattern_contains_await_expression(pattern.left.as_ref())
                    || Self::expression_contains_await_expression(&pattern.right)
            }
        }
    }

    fn expression_contains_await_expression(expression: &Expression) -> bool {
        match expression {
            Expression::Await(_) => true,
            Expression::Identifier(_)
            | Expression::PrivateIdentifier(_)
            | Expression::Literal(_)
            | Expression::This(_)
            | Expression::Super(_)
            | Expression::MetaProperty(_) => false,
            Expression::Array(expression) => expression.elements.iter().flatten().any(|element| {
                match element {
                    ArrayElement::Expression(expression) => {
                        Self::expression_contains_await_expression(expression)
                    }
                    ArrayElement::Spread { argument, .. } => {
                        Self::expression_contains_await_expression(argument)
                    }
                }
            }),
            Expression::Object(expression) => expression.properties.iter().any(|property| match property {
                ObjectProperty::Property { key, value, .. } => {
                    matches!(key, PropertyKey::Computed { expression, .. } if Self::expression_contains_await_expression(expression))
                        || Self::expression_contains_await_expression(value)
                }
                ObjectProperty::Spread { argument, .. } => {
                    Self::expression_contains_await_expression(argument)
                }
            }),
            Expression::Function(_) | Expression::ArrowFunction(_) | Expression::Class(_) => false,
            Expression::TaggedTemplate(expression) => {
                Self::expression_contains_await_expression(&expression.tag)
            }
            Expression::Yield(expression) => expression
                .argument
                .as_ref()
                .is_some_and(Self::expression_contains_await_expression),
            Expression::Unary(expression) => {
                Self::expression_contains_await_expression(&expression.argument)
            }
            Expression::Update(expression) => {
                Self::expression_contains_await_expression(&expression.argument)
            }
            Expression::Binary(expression) => {
                Self::expression_contains_await_expression(&expression.left)
                    || Self::expression_contains_await_expression(&expression.right)
            }
            Expression::Logical(expression) => {
                Self::expression_contains_await_expression(&expression.left)
                    || Self::expression_contains_await_expression(&expression.right)
            }
            Expression::Assignment(expression) => {
                Self::expression_contains_await_expression(&expression.left)
                    || Self::expression_contains_await_expression(&expression.right)
            }
            Expression::Conditional(expression) => {
                Self::expression_contains_await_expression(&expression.test)
                    || Self::expression_contains_await_expression(&expression.consequent)
                    || Self::expression_contains_await_expression(&expression.alternate)
            }
            Expression::Sequence(expression) => expression
                .expressions
                .iter()
                .any(Self::expression_contains_await_expression),
            Expression::Call(expression) => {
                Self::expression_contains_await_expression(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            Self::expression_contains_await_expression(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            Self::expression_contains_await_expression(argument)
                        }
                    })
            }
            Expression::Member(expression) => {
                Self::expression_contains_await_expression(&expression.object)
                    || matches!(
                        &expression.property,
                        MemberProperty::Computed { expression, .. }
                            if Self::expression_contains_await_expression(expression)
                    )
            }
            Expression::New(expression) => {
                Self::expression_contains_await_expression(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            Self::expression_contains_await_expression(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            Self::expression_contains_await_expression(argument)
                        }
                    })
            }
        }
    }

    fn patterns_contain_yield_expression(&self, patterns: &[Pattern]) -> bool {
        patterns.iter().any(Self::pattern_contains_yield_expression)
    }

    fn pattern_contains_yield_expression(pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Identifier(_) => false,
            Pattern::Array(pattern) => pattern
                .elements
                .iter()
                .flatten()
                .any(Self::pattern_contains_yield_expression),
            Pattern::Object(pattern) => pattern.properties.iter().any(|property| match property {
                ObjectPatternProperty::Property { value, .. } => {
                    Self::pattern_contains_yield_expression(value)
                }
                ObjectPatternProperty::Rest { argument, .. } => {
                    Self::pattern_contains_yield_expression(argument)
                }
            }),
            Pattern::Rest(pattern) => {
                Self::pattern_contains_yield_expression(pattern.argument.as_ref())
            }
            Pattern::Assignment(pattern) => {
                Self::pattern_contains_yield_expression(pattern.left.as_ref())
                    || Self::expression_contains_yield_expression(&pattern.right)
            }
        }
    }

    fn expression_contains_yield_expression(expression: &Expression) -> bool {
        match expression {
            Expression::Yield(_) => true,
            Expression::Identifier(_)
            | Expression::PrivateIdentifier(_)
            | Expression::Literal(_)
            | Expression::This(_)
            | Expression::Super(_)
            | Expression::MetaProperty(_) => false,
            Expression::Array(expression) => expression.elements.iter().flatten().any(|element| {
                match element {
                    ArrayElement::Expression(expression) => {
                        Self::expression_contains_yield_expression(expression)
                    }
                    ArrayElement::Spread { argument, .. } => {
                        Self::expression_contains_yield_expression(argument)
                    }
                }
            }),
            Expression::Object(expression) => expression.properties.iter().any(|property| match property {
                ObjectProperty::Property { key, value, .. } => {
                    matches!(key, PropertyKey::Computed { expression, .. } if Self::expression_contains_yield_expression(expression))
                        || Self::expression_contains_yield_expression(value)
                }
                ObjectProperty::Spread { argument, .. } => {
                    Self::expression_contains_yield_expression(argument)
                }
            }),
            Expression::Function(_) | Expression::ArrowFunction(_) | Expression::Class(_) => false,
            Expression::TaggedTemplate(expression) => {
                Self::expression_contains_yield_expression(&expression.tag)
            }
            Expression::Await(expression) => {
                Self::expression_contains_yield_expression(&expression.argument)
            }
            Expression::Unary(expression) => {
                Self::expression_contains_yield_expression(&expression.argument)
            }
            Expression::Update(expression) => {
                Self::expression_contains_yield_expression(&expression.argument)
            }
            Expression::Binary(expression) => {
                Self::expression_contains_yield_expression(&expression.left)
                    || Self::expression_contains_yield_expression(&expression.right)
            }
            Expression::Logical(expression) => {
                Self::expression_contains_yield_expression(&expression.left)
                    || Self::expression_contains_yield_expression(&expression.right)
            }
            Expression::Assignment(expression) => {
                Self::expression_contains_yield_expression(&expression.left)
                    || Self::expression_contains_yield_expression(&expression.right)
            }
            Expression::Conditional(expression) => {
                Self::expression_contains_yield_expression(&expression.test)
                    || Self::expression_contains_yield_expression(&expression.consequent)
                    || Self::expression_contains_yield_expression(&expression.alternate)
            }
            Expression::Sequence(expression) => expression
                .expressions
                .iter()
                .any(Self::expression_contains_yield_expression),
            Expression::Call(expression) => {
                Self::expression_contains_yield_expression(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            Self::expression_contains_yield_expression(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            Self::expression_contains_yield_expression(argument)
                        }
                    })
            }
            Expression::Member(expression) => {
                Self::expression_contains_yield_expression(&expression.object)
                    || matches!(
                        &expression.property,
                        MemberProperty::Computed { expression, .. }
                            if Self::expression_contains_yield_expression(expression)
                    )
            }
            Expression::New(expression) => {
                Self::expression_contains_yield_expression(&expression.callee)
                    || expression.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            Self::expression_contains_yield_expression(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            Self::expression_contains_yield_expression(argument)
                        }
                    })
            }
        }
    }

    fn is_valid_update_target(&self, expression: &Expression, strict: bool) -> bool {
        self.is_valid_simple_target(expression, strict)
    }

    fn is_valid_assignment_target(
        &self,
        expression: &Expression,
        allow_pattern: bool,
        strict: bool,
    ) -> bool {
        self.is_valid_simple_target(expression, strict)
            || (allow_pattern && matches!(expression, Expression::Array(_) | Expression::Object(_)))
    }

    fn is_valid_for_each_left_target(&self, expression: &Expression, strict: bool) -> bool {
        self.is_valid_simple_target(expression, strict)
            || matches!(expression, Expression::Array(_) | Expression::Object(_))
    }

    fn is_valid_logical_assignment_target(&self, expression: &Expression) -> bool {
        matches!(expression, Expression::Identifier(_))
            || matches!(
                expression,
                Expression::Member(member)
                    if !member.optional && !self.is_import_meta_expression(expression)
            )
    }

    fn is_valid_simple_target(&self, expression: &Expression, strict: bool) -> bool {
        match expression {
            Expression::Identifier(_) => true,
            Expression::Member(member) => {
                !member.optional && !self.is_import_meta_expression(expression)
            }
            Expression::Call(call) => {
                !strict && !call.optional && self.import_callee_kind(&call.callee).is_none()
            }
            _ => false,
        }
    }

    fn statement_list_has_use_strict(&self, statements: &[Statement]) -> bool {
        statements
            .iter()
            .take_while(|statement| self.directive_value(statement).is_some())
            .any(|statement| self.directive_value(statement) == Some("use strict"))
    }

    fn directive_value<'a>(&self, statement: &'a Statement) -> Option<&'a str> {
        match statement {
            Statement::Directive(directive) => Some(directive.value.as_str()),
            Statement::Expression(statement) => match &statement.expression {
                Expression::Literal(Literal::String(string)) => Some(string.value.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    fn is_strict_reserved_word(&self, name: &str) -> bool {
        matches!(
            name,
            "implements"
                | "interface"
                | "let"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "static"
                | "yield"
        )
    }

    fn is_simple_parameter_list(&self, params: &[Pattern]) -> bool {
        params
            .iter()
            .all(|pattern| matches!(pattern, Pattern::Identifier(_)))
    }

    fn first_duplicate_bound_name(&self, params: &[Pattern]) -> Option<Identifier> {
        let mut names = HashSet::new();
        for param in params {
            if let Some(identifier) = Self::record_bound_names(param, &mut names) {
                return Some(identifier);
            }
        }
        None
    }

    fn record_bound_names(pattern: &Pattern, names: &mut HashSet<String>) -> Option<Identifier> {
        match pattern {
            Pattern::Identifier(identifier) => {
                if names.insert(identifier.name.clone()) {
                    None
                } else {
                    Some(identifier.clone())
                }
            }
            Pattern::Array(pattern) => {
                for element in pattern.elements.iter().flatten() {
                    if let Some(identifier) = Self::record_bound_names(element, names) {
                        return Some(identifier);
                    }
                }
                None
            }
            Pattern::Object(pattern) => {
                for property in &pattern.properties {
                    match property {
                        ObjectPatternProperty::Property { value, .. } => {
                            if let Some(identifier) = Self::record_bound_names(value, names) {
                                return Some(identifier);
                            }
                        }
                        ObjectPatternProperty::Rest { argument, .. } => {
                            if let Some(identifier) = Self::record_bound_names(argument, names) {
                                return Some(identifier);
                            }
                        }
                    }
                }
                None
            }
            Pattern::Rest(pattern) => Self::record_bound_names(pattern.argument.as_ref(), names),
            Pattern::Assignment(pattern) => Self::record_bound_names(pattern.left.as_ref(), names),
        }
    }

    fn current_is_identifier_named(&self, name: &str) -> bool {
        !self.current().escaped && self.current().kind.identifier() == Some(name)
    }

    fn identifier_was_escaped(&self, identifier: &Identifier) -> bool {
        self.escaped_identifiers.contains(&identifier.span)
    }

    fn is_always_reserved_identifier_name(&self, name: &str) -> bool {
        matches!(
            name,
            "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "function"
                | "if"
                | "import"
                | "in"
                | "instanceof"
                | "new"
                | "null"
                | "return"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "typeof"
                | "var"
                | "void"
                | "while"
                | "with"
        )
    }

    fn escaped_identifier_is_reserved_here(
        &self,
        name: &str,
        strict: bool,
        await_reserved: bool,
        yield_reserved: bool,
    ) -> bool {
        self.is_always_reserved_identifier_name(name)
            || (await_reserved && name == "await")
            || (yield_reserved && name == "yield")
            || (strict && self.is_strict_reserved_word(name))
    }

    fn eat_contextual(&mut self, name: &str) -> bool {
        if self.current_is_identifier_named(name) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_contextual(&mut self, name: &str) -> Result<Token, ParseError> {
        if self.current_is_identifier_named(name) {
            Ok(self.advance())
        } else {
            Err(ParseError::new(
                format!("expected {name}, found '{}'", self.current().kind),
                self.current().span,
            ))
        }
    }

    fn at(&self, tag: TokenTag) -> bool {
        self.current_tag() == tag
    }

    fn eat(&mut self, tag: TokenTag) -> bool {
        if self.at(tag) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tag: TokenTag) -> Result<Token, ParseError> {
        if self.at(tag) {
            Ok(self.advance())
        } else {
            Err(ParseError::new(
                format!("expected {tag}, found '{}'", self.current().kind),
                self.current().span,
            ))
        }
    }

    fn expect_span(&mut self, tag: TokenTag) -> Result<Span, ParseError> {
        if self.at(tag) {
            Ok(self.advance_span())
        } else {
            Err(ParseError::new(
                format!("expected {tag}, found '{}'", self.current().kind),
                self.current().span,
            ))
        }
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        if !self.at(TokenTag::Eof) {
            self.index += 1;
        }
        token
    }

    fn advance_span(&mut self) -> Span {
        let span = self.current().span;
        if !self.at(TokenTag::Eof) {
            self.index += 1;
        }
        span
    }

    fn rewind_one(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.index]
    }

    fn current_tag(&self) -> TokenTag {
        self.current().tag()
    }

    fn previous(&self) -> &Token {
        if self.index == 0 {
            &self.tokens[0]
        } else {
            &self.tokens[self.index - 1]
        }
    }

    fn peek(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.index + offset)
    }

    fn peek_tag(&self, offset: usize) -> Option<TokenTag> {
        self.peek(offset).map(Token::tag)
    }

    fn error_current(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(message, self.current().span)
    }
}

#[derive(Debug, Clone, Copy)]
struct ValidationContext {
    strict: bool,
    await_reserved: bool,
    yield_reserved: bool,
    in_class_static_block: bool,
    module_await_reserved: bool,
    return_allowed: bool,
    new_target_allowed: bool,
    super_property_allowed: bool,
    super_call_allowed: bool,
}

impl ValidationContext {
    fn with_strict(self, strict: bool) -> Self {
        Self { strict, ..self }
    }

    fn for_function_name(self, is_expression: bool, is_async: bool, is_generator: bool) -> Self {
        Self {
            strict: self.strict,
            await_reserved: self.module_await_reserved
                || (is_expression && is_async && is_generator),
            yield_reserved: is_expression && is_generator,
            in_class_static_block: self.in_class_static_block && !is_expression,
            module_await_reserved: self.module_await_reserved,
            return_allowed: false,
            new_target_allowed: false,
            super_property_allowed: false,
            super_call_allowed: false,
        }
    }

    fn for_function(self, is_async: bool, is_generator: bool, has_use_strict: bool) -> Self {
        Self {
            strict: self.strict || has_use_strict,
            await_reserved: self.module_await_reserved || is_async,
            yield_reserved: is_generator,
            in_class_static_block: false,
            module_await_reserved: self.module_await_reserved,
            return_allowed: true,
            new_target_allowed: true,
            super_property_allowed: false,
            super_call_allowed: false,
        }
    }

    fn for_arrow_function(self, is_async: bool, has_use_strict: bool) -> Self {
        Self {
            strict: self.strict || has_use_strict,
            await_reserved: self.module_await_reserved || self.await_reserved || is_async,
            yield_reserved: self.yield_reserved,
            in_class_static_block: self.in_class_static_block,
            module_await_reserved: self.module_await_reserved,
            return_allowed: true,
            new_target_allowed: self.new_target_allowed,
            super_property_allowed: self.super_property_allowed,
            super_call_allowed: self.super_call_allowed,
        }
    }

    fn for_class_element_body(self) -> Self {
        Self {
            strict: true,
            await_reserved: self.module_await_reserved,
            yield_reserved: false,
            in_class_static_block: false,
            module_await_reserved: self.module_await_reserved,
            return_allowed: false,
            new_target_allowed: true,
            super_property_allowed: true,
            super_call_allowed: false,
        }
    }

    fn with_class_static_block(self, in_class_static_block: bool) -> Self {
        Self {
            in_class_static_block,
            ..self
        }
    }

    fn with_super_capabilities(
        self,
        super_property_allowed: bool,
        super_call_allowed: bool,
    ) -> Self {
        Self {
            super_property_allowed,
            super_call_allowed,
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatementListKind {
    Script,
    Module,
    FunctionBody,
    Block,
    SwitchCase,
    StaticBlock,
}

impl StatementListKind {
    fn allow_module_declarations(self) -> bool {
        matches!(self, Self::Module)
    }

    fn function_declarations_are_lexical(self) -> bool {
        matches!(
            self,
            Self::Module | Self::Block | Self::SwitchCase | Self::StaticBlock
        )
    }

    fn function_declarations_are_var(self) -> bool {
        matches!(self, Self::Script | Self::FunctionBody)
    }
}

#[derive(Debug, Clone)]
struct LabelContext {
    name: String,
    continue_allowed: bool,
}

#[derive(Debug, Clone)]
struct NamedSpan {
    name: String,
    span: Span,
}

#[derive(Debug, Clone)]
struct LexicalName {
    binding: NamedSpan,
    function_declaration: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportCallKind {
    Dynamic,
    Source,
    Defer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateNameDeclarationKind {
    Getter,
    Setter,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct PrivateNameDeclaration {
    is_static: bool,
    kind: PrivateNameDeclarationKind,
    span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfixOperator {
    Binary(BinaryOperator),
    Logical(LogicalOperator),
}

trait ArgumentSpan {
    fn span(&self) -> Span;
}

impl ArgumentSpan for CallArgument {
    fn span(&self) -> Span {
        match self {
            Self::Expression(expression) => expression.span(),
            Self::Spread { span, .. } => *span,
        }
    }
}

trait MemberPropertySpan {
    fn span(&self) -> Span;
}

impl MemberPropertySpan for MemberProperty {
    fn span(&self) -> Span {
        match self {
            Self::Identifier(identifier) | Self::PrivateName(identifier) => identifier.span,
            Self::Computed { span, .. } => *span,
        }
    }
}

fn is_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}
