mod compile_expressions;
mod compile_functions;
mod compile_statements;
mod emit;
mod js_value;
mod vm;

pub use crate::vm::Opcode;

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt;

use ast::{
    ArrowBody, ArrowFunction, AssignmentExpression, AssignmentOperator, BinaryExpression,
    BinaryOperator, BlockStatement, CallArgument, CallExpression, ConditionalExpression,
    DoWhileStatement, Expression, ExpressionStatement, ForClassicStatement, ForInit, ForStatement,
    Function, Identifier, IfStatement, Literal, LogicalExpression, LogicalOperator,
    MemberExpression, MemberProperty, NewExpression, NumberLiteral, ObjectExpression,
    ObjectProperty, ObjectPropertyKind, Pattern, Program, PropertyKey, ReturnStatement,
    SequenceExpression, Span, Statement, StringLiteral, UnaryExpression, UnaryOperator,
    UpdateExpression, UpdateOperator, VariableDeclaration, VariableDeclarator, WhileStatement,
};

use crate::emit::BytecodeBuilder;
use crate::js_value::{JSValue, make_number, make_undefined};

const ACC: u8 = 255;
const MAX_TEMP_REG: u8 = ACC - 1;

#[derive(Debug, Clone)]
pub struct CompiledBytecode {
    pub bytecode: Vec<u32>,
    pub constants: Vec<JSValue>,
    pub string_constants: Vec<(u16, String)>,
    pub function_constants: Vec<u16>,
    pub names: Vec<String>,
    pub properties: Vec<String>,
}

#[derive(Debug)]
pub enum CodegenError {
    Parse(ast::ParseError),
    Unsupported { feature: &'static str, span: Span },
    RegisterOverflow { span: Option<Span> },
    NameOverflow { name: String },
    PropertyOverflow { name: String },
    NumericLiteral { raw: String, span: Span },
    InvalidBreak { span: Span },
    InvalidContinue { span: Span },
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(f, "{error}"),
            Self::Unsupported { feature, .. } => write!(f, "unsupported AST feature: {feature}"),
            Self::RegisterOverflow { .. } => write!(f, "temporary register overflow"),
            Self::NameOverflow { name } => {
                write!(f, "too many bound identifiers, cannot encode `{name}`")
            }
            Self::PropertyOverflow { name } => {
                write!(f, "too many property names, cannot encode `{name}`")
            }
            Self::NumericLiteral { raw, .. } => write!(f, "invalid numeric literal `{raw}`"),
            Self::InvalidBreak { .. } => write!(f, "`break` used outside of a loop"),
            Self::InvalidContinue { .. } => write!(f, "`continue` used outside of a loop"),
        }
    }
}

impl Error for CodegenError {}

impl From<ast::ParseError> for CodegenError {
    fn from(value: ast::ParseError) -> Self {
        Self::Parse(value)
    }
}

#[derive(Debug, Clone)]
enum PendingFunctionBody {
    Function(Function),
    Arrow(ArrowFunction),
}

#[derive(Debug, Clone)]
struct PendingFunction {
    const_index: u16,
    body: PendingFunctionBody,
}

type FastNameScopeEntry = (String, Option<u8>, Option<bool>);

#[derive(Debug, Clone, Copy)]
enum JumpPatchKind {
    Jmp,
    JmpFalse { reg: u8 },
    JmpLteFalse { lhs: u8, rhs: u8 },
    Try,
}

#[derive(Debug, Clone, Copy)]
struct JumpPatch {
    pos: usize,
    target: usize,
    kind: JumpPatchKind,
}

#[derive(Debug, Default)]
struct ControlContext {
    break_patches: Vec<usize>,
    continue_patches: Option<Vec<usize>>,
}

impl ControlContext {
    fn loop_context() -> Self {
        Self {
            break_patches: Vec::new(),
            continue_patches: Some(Vec::new()),
        }
    }

    fn switch_context() -> Self {
        Self {
            break_patches: Vec::new(),
            continue_patches: None,
        }
    }
}

pub fn compile_source(source: &str) -> Result<CompiledBytecode, CodegenError> {
    let program = ast::parse(source)?;
    compile_program(&program)
}

pub fn compile_program(program: &Program) -> Result<CompiledBytecode, CodegenError> {
    let mut codegen = Codegen::new();
    let last_value = codegen.compile_statement_list(&program.body, true)?;
    codegen.finish_root(last_value);
    codegen.compile_pending_functions()?;
    codegen.finalize()
}

struct Codegen {
    builder: BytecodeBuilder,
    name_slots: HashMap<String, u16>,
    property_slots: HashMap<String, u8>,
    fast_name_regs: HashMap<String, u8>,
    fast_name_runtime_slots: HashMap<String, bool>,
    fast_name_scope_stack: Vec<Vec<FastNameScopeEntry>>,
    fast_name_scope_runtime_stack: Vec<bool>,
    names: Vec<String>,
    properties: Vec<String>,
    string_constants: Vec<(u16, String)>,
    undefined_const: Option<u16>,
    jump_patches: Vec<JumpPatch>,
    function_patches: Vec<(u16, usize)>,
    pending_functions: VecDeque<PendingFunction>,
    temp_top: u8,
    control_stack: Vec<ControlContext>,
    nested_scope_depth: usize,
    fast_name_bindings_enabled: bool,
    current_self_upvalue: Option<(String, u8)>,
    current_self_upvalue_reg: Option<u8>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            builder: BytecodeBuilder::new(),
            name_slots: HashMap::new(),
            property_slots: HashMap::new(),
            fast_name_regs: HashMap::new(),
            fast_name_runtime_slots: HashMap::new(),
            fast_name_scope_stack: Vec::new(),
            fast_name_scope_runtime_stack: Vec::new(),
            names: Vec::new(),
            properties: Vec::new(),
            string_constants: Vec::new(),
            undefined_const: None,
            jump_patches: Vec::new(),
            function_patches: Vec::new(),
            pending_functions: VecDeque::new(),
            temp_top: 0,
            control_stack: Vec::new(),
            nested_scope_depth: 0,
            fast_name_bindings_enabled: false,
            current_self_upvalue: None,
            current_self_upvalue_reg: None,
        }
    }

    fn finalize(self) -> Result<CompiledBytecode, CodegenError> {
        let (mut bytecode, mut constants) = self.builder.build();

        for patch in self.jump_patches {
            let offset = patch.target as isize - patch.pos as isize - 1;
            let offset = i16::try_from(offset).map_err(|_| CodegenError::Unsupported {
                feature: "jump offset out of range",
                span: Span::default(),
            })?;
            bytecode[patch.pos] = match patch.kind {
                JumpPatchKind::Jmp => encode_asbx(Opcode::Jmp, 0, offset),
                JumpPatchKind::JmpFalse { reg } => encode_asbx(Opcode::JmpFalse, reg, offset),
                JumpPatchKind::JmpLteFalse { lhs, rhs } => {
                    let short = i8::try_from(offset).map_err(|_| CodegenError::Unsupported {
                        feature: "short compare jump offset out of range",
                        span: Span::default(),
                    })?;
                    ((short as u8 as u32) << 24)
                        | ((rhs as u32) << 16)
                        | ((lhs as u32) << 8)
                        | Opcode::JmpLteFalse.as_u8() as u32
                }
                JumpPatchKind::Try => encode_asbx(Opcode::Try, 0, offset),
            };
        }

        let function_constants = self
            .function_patches
            .iter()
            .map(|(const_index, _)| *const_index)
            .collect::<Vec<_>>();

        for (const_index, entry_pc) in self.function_patches {
            if let Some(slot) = constants.get_mut(const_index as usize) {
                *slot = make_number(entry_pc as f64);
            }
        }

        Ok(CompiledBytecode {
            bytecode,
            constants,
            string_constants: self.string_constants,
            function_constants,
            names: self.names,
            properties: self.properties,
        })
    }

    fn finish_root(&mut self, last_value: Option<u8>) {
        if let Some(reg) = last_value {
            self.builder.emit_mov(ACC, reg);
            self.builder.emit_ret();
        } else {
            self.builder.emit_ret_u();
        }
    }

    fn load_runtime_string(&mut self, value: &str, span: Span) -> Result<u8, CodegenError> {
        let reg = self.alloc_temp(Some(span))?;
        let index = self.builder.add_constant(make_undefined());
        self.string_constants.push((index, value.to_owned()));
        self.builder.emit_load_k(reg, index);
        Ok(reg)
    }

    fn load_undefined(&mut self, span: Option<Span>) -> Result<u8, CodegenError> {
        let reg = self.alloc_temp(span)?;
        let index = *self
            .undefined_const
            .get_or_insert_with(|| self.builder.add_constant(make_undefined()));
        self.builder.emit_load_k(reg, index);
        Ok(reg)
    }

    fn reserve_function_constant(&mut self) -> u16 {
        self.builder.add_constant(make_number(0.0))
    }

    fn declare_name_binding(&mut self, name: &str, value_reg: u8) -> Result<(), CodegenError> {
        let slot = self.name_slot(name)?;
        if self.current_fast_name_scope_requires_runtime_bindings() {
            self.builder.emit_init_name(value_reg, slot);
        }
        self.promote_fast_name(name, value_reg);
        Ok(())
    }

    fn write_name_binding(&mut self, name: &str, value_reg: u8) -> Result<(), CodegenError> {
        let slot = self.name_slot(name)?;
        if self.should_emit_runtime_name_ops(name) {
            self.builder.emit_store_name(value_reg, slot);
        }
        self.sync_fast_name(name, value_reg);
        Ok(())
    }

    fn alloc_temp(&mut self, span: Option<Span>) -> Result<u8, CodegenError> {
        loop {
            if self.temp_top >= MAX_TEMP_REG {
                return Err(CodegenError::RegisterOverflow { span });
            }
            self.temp_top += 1;
            if !self
                .fast_name_regs
                .values()
                .any(|&reg| reg == self.temp_top)
                && self.current_self_upvalue_reg != Some(self.temp_top)
            {
                return Ok(self.temp_top);
            }
        }
    }

    fn emit_placeholder_jmp(&mut self) -> usize {
        let pos = self.builder.len();
        self.builder.emit_jmp(0);
        pos
    }

    fn emit_placeholder_jmp_false(&mut self, reg: u8) -> usize {
        let pos = self.builder.len();
        self.builder.emit_jmp_false(reg, 0);
        pos
    }

    fn emit_placeholder_jmp_lte_false(&mut self, lhs: u8, rhs: u8) -> usize {
        let pos = self.builder.len();
        self.builder.emit_jmp_lte_false(lhs, rhs, 0);
        pos
    }
    fn patch_jump(&mut self, pos: usize, target: usize, kind: JumpPatchKind) {
        self.jump_patches.push(JumpPatch { pos, target, kind });
    }

    fn patch_loop_breaks(&mut self, patches: Vec<usize>, target: usize) {
        for pos in patches {
            self.patch_jump(pos, target, JumpPatchKind::Jmp);
        }
    }

    fn patch_loop_continues(&mut self, patches: Vec<usize>, target: usize) {
        for pos in patches {
            self.patch_jump(pos, target, JumpPatchKind::Jmp);
        }
    }

    fn name_slot(&mut self, name: &str) -> Result<u16, CodegenError> {
        if let Some(&slot) = self.name_slots.get(name) {
            return Ok(slot);
        }

        let slot =
            u16::try_from(self.name_slots.len()).map_err(|_| CodegenError::NameOverflow {
                name: name.to_owned(),
            })?;
        self.name_slots.insert(name.to_owned(), slot);
        self.names.push(name.to_owned());
        Ok(slot)
    }

    fn property_slot(&mut self, name: &str) -> Result<u8, CodegenError> {
        if let Some(&slot) = self.property_slots.get(name) {
            return Ok(slot);
        }

        let slot = u8::try_from(self.property_slots.len()).map_err(|_| {
            CodegenError::PropertyOverflow {
                name: name.to_owned(),
            }
        })?;
        self.property_slots.insert(name.to_owned(), slot);
        self.properties.push(name.to_owned());
        Ok(slot)
    }

    fn fast_name_reg(&self, name: &str) -> Option<u8> {
        if !self.fast_name_bindings_enabled {
            return None;
        }
        self.fast_name_regs.get(name).copied()
    }

    fn enter_fast_name_scope_with_runtime_bindings(&mut self, requires_runtime_bindings: bool) {
        if self.fast_name_bindings_enabled {
            self.fast_name_scope_stack.push(Vec::new());
            self.fast_name_scope_runtime_stack
                .push(requires_runtime_bindings);
        }
    }

    fn leave_fast_name_scope(&mut self) {
        if !self.fast_name_bindings_enabled {
            return;
        }

        let Some(mut bindings) = self.fast_name_scope_stack.pop() else {
            return;
        };
        let _ = self.fast_name_scope_runtime_stack.pop();

        while let Some((name, previous_reg, previous_runtime_slot)) = bindings.pop() {
            if let Some(reg) = previous_reg {
                self.fast_name_regs.insert(name.clone(), reg);
            } else {
                self.fast_name_regs.remove(&name);
            }

            if let Some(requires_runtime_slot) = previous_runtime_slot {
                self.fast_name_runtime_slots
                    .insert(name, requires_runtime_slot);
            } else {
                self.fast_name_runtime_slots.remove(&name);
            }
        }
    }

    fn current_fast_name_scope_requires_runtime_bindings(&self) -> bool {
        self.fast_name_scope_runtime_stack
            .last()
            .copied()
            .unwrap_or(true)
    }

    fn fast_name_requires_runtime_slot(&self, name: &str) -> bool {
        self.fast_name_runtime_slots
            .get(name)
            .copied()
            .unwrap_or(true)
    }

    fn should_emit_runtime_name_ops(&self, name: &str) -> bool {
        self.fast_name_reg(name).is_none() || self.fast_name_requires_runtime_slot(name)
    }

    fn record_fast_name_scope_change(&mut self, name: &str) {
        if !self.fast_name_bindings_enabled {
            return;
        }

        let Some(scope) = self.fast_name_scope_stack.last_mut() else {
            return;
        };
        if scope.iter().any(|(existing, _, _)| existing == name) {
            return;
        }

        scope.push((
            name.to_owned(),
            self.fast_name_regs.get(name).copied(),
            self.fast_name_runtime_slots.get(name).copied(),
        ));
    }

    fn promote_fast_name(&mut self, name: &str, value_reg: u8) {
        if !self.fast_name_bindings_enabled {
            return;
        }

        self.record_fast_name_scope_change(name);
        self.fast_name_regs.insert(name.to_owned(), value_reg);
        self.fast_name_runtime_slots.insert(
            name.to_owned(),
            self.current_fast_name_scope_requires_runtime_bindings(),
        );
    }

    fn sync_fast_name(&mut self, name: &str, value_reg: u8) {
        let Some(home_reg) = self.fast_name_reg(name) else {
            return;
        };
        if home_reg != value_reg {
            self.builder.emit_mov(home_reg, value_reg);
            self.temp_top = self.temp_top.max(home_reg);
        }
    }

    fn block_needs_runtime_bindings(&self, block: &BlockStatement) -> bool {
        // Check if any statement in the block requires runtime bindings
        block
            .body
            .iter()
            .any(|stmt| self.statement_needs_runtime_bindings(stmt))
    }

    fn statement_needs_runtime_bindings(&self, statement: &Statement) -> bool {
        match statement {
            Statement::Directive(_) | Statement::Empty(_) | Statement::Debugger(_) => false,
            Statement::Block(block) => self.block_needs_runtime_bindings(block),
            Statement::VariableDeclaration(declaration) => declaration
                .declarations
                .iter()
                .any(|declarator| self.variable_declarator_needs_runtime_bindings(declarator)),
            Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => true,
            Statement::If(statement) => {
                self.expression_needs_runtime_bindings(&statement.test)
                    || self.statement_needs_runtime_bindings(&statement.consequent)
                    || statement
                        .alternate
                        .as_ref()
                        .is_some_and(|alternate| self.statement_needs_runtime_bindings(alternate))
            }
            Statement::While(statement) => {
                self.expression_needs_runtime_bindings(&statement.test)
                    || self.statement_needs_runtime_bindings(&statement.body)
            }
            Statement::DoWhile(statement) => {
                self.statement_needs_runtime_bindings(&statement.body)
                    || self.expression_needs_runtime_bindings(&statement.test)
            }
            Statement::For(statement) => self.for_statement_needs_runtime_bindings(statement),
            Statement::Return(statement) => statement
                .argument
                .as_ref()
                .is_some_and(|expr| self.expression_needs_runtime_bindings(expr)),
            Statement::Break(_) | Statement::Continue(_) => false,
            Statement::Expression(statement) => {
                self.expression_needs_runtime_bindings(&statement.expression)
            }
            Statement::Switch(statement) => {
                self.expression_needs_runtime_bindings(&statement.discriminant)
                    || statement.cases.iter().any(|case| {
                        case.test
                            .as_ref()
                            .is_some_and(|test| self.expression_needs_runtime_bindings(test))
                            || case
                                .consequent
                                .iter()
                                .any(|stmt| self.statement_needs_runtime_bindings(stmt))
                    })
            }
            Statement::Throw(statement) => {
                self.expression_needs_runtime_bindings(&statement.argument)
            }
            Statement::Try(statement) => {
                statement
                    .block
                    .body
                    .iter()
                    .any(|stmt| self.statement_needs_runtime_bindings(stmt))
                    || statement.handler.as_ref().is_some_and(|handler| {
                        handler
                            .param
                            .as_ref()
                            .is_some_and(|param| self.pattern_needs_runtime_bindings(param))
                            || handler
                                .body
                                .body
                                .iter()
                                .any(|stmt| self.statement_needs_runtime_bindings(stmt))
                    })
                    || statement.finalizer.as_ref().is_some_and(|block| {
                        block
                            .body
                            .iter()
                            .any(|stmt| self.statement_needs_runtime_bindings(stmt))
                    })
            }
            Statement::Labeled(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportDeclaration(_)
            | Statement::With(_) => true,
        }
    }

    fn for_statement_needs_runtime_bindings(&self, statement: &ForStatement) -> bool {
        match statement {
            ForStatement::Classic(statement) => {
                statement.init.as_ref().is_some_and(|init| match init {
                    ForInit::VariableDeclaration(declaration) => {
                        declaration.declarations.iter().any(|declarator| {
                            self.variable_declarator_needs_runtime_bindings(declarator)
                        })
                    }
                    ForInit::Expression(expression) => {
                        self.expression_needs_runtime_bindings(expression)
                    }
                }) || statement
                    .test
                    .as_ref()
                    .is_some_and(|test| self.expression_needs_runtime_bindings(test))
                    || statement
                        .update
                        .as_ref()
                        .is_some_and(|update| self.expression_needs_runtime_bindings(update))
                    || self.statement_needs_runtime_bindings(&statement.body)
            }
            ForStatement::In(statement) => {
                self.for_left_needs_runtime_bindings(&statement.left)
                    || self.expression_needs_runtime_bindings(&statement.right)
                    || self.statement_needs_runtime_bindings(&statement.body)
            }
            ForStatement::Of(statement) => {
                self.for_left_needs_runtime_bindings(&statement.left)
                    || self.expression_needs_runtime_bindings(&statement.right)
                    || self.statement_needs_runtime_bindings(&statement.body)
            }
        }
    }

    fn for_left_needs_runtime_bindings(&self, left: &ast::ForLeft) -> bool {
        match left {
            ast::ForLeft::VariableDeclaration(declaration) => declaration
                .declarations
                .iter()
                .any(|declarator| self.variable_declarator_needs_runtime_bindings(declarator)),
            ast::ForLeft::Pattern(pattern) => self.pattern_needs_runtime_bindings(pattern),
            ast::ForLeft::Expression(expression) => {
                self.expression_needs_runtime_bindings(expression)
            }
        }
    }

    fn variable_declarator_needs_runtime_bindings(&self, declarator: &VariableDeclarator) -> bool {
        self.pattern_needs_runtime_bindings(&declarator.pattern)
            || declarator
                .init
                .as_ref()
                .is_some_and(|init| self.expression_needs_runtime_bindings(init))
    }

    fn pattern_needs_runtime_bindings(&self, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Identifier(_) => false,
            Pattern::Assignment(pattern) => {
                self.pattern_needs_runtime_bindings(&pattern.left)
                    || self.expression_needs_runtime_bindings(&pattern.right)
            }
            _ => true,
        }
    }

    fn expression_needs_runtime_bindings(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Identifier(_) | Expression::Literal(_) | Expression::This(_) => false,
            Expression::Array(array) => {
                array
                    .elements
                    .iter()
                    .flatten()
                    .any(|element| match element {
                        ast::ArrayElement::Expression(expression) => {
                            self.expression_needs_runtime_bindings(expression)
                        }
                        ast::ArrayElement::Spread { argument, .. } => {
                            self.expression_needs_runtime_bindings(argument)
                        }
                    })
            }
            Expression::Object(object) => object
                .properties
                .iter()
                .any(|property| self.object_property_needs_runtime_bindings(property)),
            Expression::Function(_) | Expression::ArrowFunction(_) | Expression::Class(_) => true,
            Expression::Unary(unary) => self.expression_needs_runtime_bindings(&unary.argument),
            Expression::Update(update) => self.expression_needs_runtime_bindings(&update.argument),
            Expression::Binary(binary) => {
                self.expression_needs_runtime_bindings(&binary.left)
                    || self.expression_needs_runtime_bindings(&binary.right)
            }
            Expression::Logical(binary) => {
                self.expression_needs_runtime_bindings(&binary.left)
                    || self.expression_needs_runtime_bindings(&binary.right)
            }
            Expression::Assignment(assignment) => {
                self.expression_needs_runtime_bindings(&assignment.left)
                    || self.expression_needs_runtime_bindings(&assignment.right)
            }
            Expression::Conditional(conditional) => {
                self.expression_needs_runtime_bindings(&conditional.test)
                    || self.expression_needs_runtime_bindings(&conditional.consequent)
                    || self.expression_needs_runtime_bindings(&conditional.alternate)
            }
            Expression::Sequence(sequence) => sequence
                .expressions
                .iter()
                .any(|expr| self.expression_needs_runtime_bindings(expr)),
            Expression::Call(call) => {
                self.expression_needs_runtime_bindings(&call.callee)
                    || call.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            self.expression_needs_runtime_bindings(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.expression_needs_runtime_bindings(argument)
                        }
                    })
            }
            Expression::New(new_expr) => {
                self.expression_needs_runtime_bindings(&new_expr.callee)
                    || new_expr.arguments.iter().any(|argument| match argument {
                        CallArgument::Expression(expression) => {
                            self.expression_needs_runtime_bindings(expression)
                        }
                        CallArgument::Spread { argument, .. } => {
                            self.expression_needs_runtime_bindings(argument)
                        }
                    })
            }
            Expression::Member(member) => {
                self.expression_needs_runtime_bindings(&member.object)
                    || match &member.property {
                        MemberProperty::Identifier(_) | MemberProperty::PrivateName(_) => false,
                        MemberProperty::Computed { expression, .. } => {
                            self.expression_needs_runtime_bindings(expression)
                        }
                    }
            }
            _ => true,
        }
    }

    fn object_property_needs_runtime_bindings(&self, property: &ObjectProperty) -> bool {
        match property {
            ObjectProperty::Property {
                key,
                value,
                shorthand: _,
                kind: _,
                span: _,
            } => {
                self.property_key_needs_runtime_bindings(key)
                    || self.expression_needs_runtime_bindings(value)
            }
            ObjectProperty::Spread { argument, .. } => {
                self.expression_needs_runtime_bindings(argument)
            }
        }
    }

    fn property_key_needs_runtime_bindings(&self, key: &PropertyKey) -> bool {
        match key {
            PropertyKey::Identifier(_)
            | PropertyKey::PrivateName(_)
            | PropertyKey::String(_)
            | PropertyKey::Number(_) => false,
            PropertyKey::Computed { expression, .. } => {
                self.expression_needs_runtime_bindings(expression)
            }
        }
    }
}

fn parse_number_literal(literal: &NumberLiteral) -> Result<f64, CodegenError> {
    let raw = literal.raw.replace('_', "");
    let value = if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok().map(|value| value as f64)
    } else if let Some(bin) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
        i64::from_str_radix(bin, 2).ok().map(|value| value as f64)
    } else if let Some(oct) = raw.strip_prefix("0o").or_else(|| raw.strip_prefix("0O")) {
        i64::from_str_radix(oct, 8).ok().map(|value| value as f64)
    } else {
        raw.parse::<f64>().ok()
    };

    value.ok_or_else(|| CodegenError::NumericLiteral {
        raw: literal.raw.clone(),
        span: literal.span,
    })
}

fn extract_call1_sub_i_arg(
    arguments: &[CallArgument],
) -> Result<Option<(&Expression, i8)>, CodegenError> {
    let [CallArgument::Expression(Expression::Binary(binary))] = arguments else {
        return Ok(None);
    };
    if binary.operator != BinaryOperator::Subtract {
        return Ok(None);
    }
    let Expression::Literal(Literal::Number(number)) = &binary.right else {
        return Ok(None);
    };
    let value = parse_number_literal(number)?;
    if value.fract() != 0.0 || value < i8::MIN as f64 || value > i8::MAX as f64 {
        return Ok(None);
    }

    Ok(Some((&binary.left, value as i8)))
}

fn pending_function_requires_runtime_env(pending: &PendingFunctionBody) -> bool {
    match pending {
        PendingFunctionBody::Function(function) => function_requires_runtime_env(function),
        PendingFunctionBody::Arrow(function) => arrow_function_requires_runtime_env(function),
    }
}

fn function_requires_runtime_env(function: &Function) -> bool {
    function.params.iter().any(pattern_requires_runtime_env)
        || statement_requires_runtime_env(&Statement::Block(function.body.clone()))
}

fn arrow_function_requires_runtime_env(function: &ArrowFunction) -> bool {
    function.params.iter().any(pattern_requires_runtime_env)
        || match &function.body {
            ArrowBody::Expression(expression) => expression_requires_runtime_env(expression),
            ArrowBody::Block(block) => {
                statement_requires_runtime_env(&Statement::Block(block.clone()))
            }
        }
}

fn pattern_requires_runtime_env(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Identifier(_) => false,
        Pattern::Assignment(pattern) => {
            pattern_requires_runtime_env(&pattern.left)
                || expression_requires_runtime_env(&pattern.right)
        }
        _ => true,
    }
}

fn statement_requires_runtime_env(statement: &Statement) -> bool {
    match statement {
        Statement::Directive(_) | Statement::Empty(_) | Statement::Debugger(_) => false,
        Statement::Block(block) => block.body.iter().any(statement_requires_runtime_env),
        Statement::VariableDeclaration(declaration) => declaration
            .declarations
            .iter()
            .any(variable_declarator_requires_runtime_env),
        Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => true,
        Statement::If(statement) => {
            expression_requires_runtime_env(&statement.test)
                || statement_requires_runtime_env(&statement.consequent)
                || statement
                    .alternate
                    .as_ref()
                    .is_some_and(|alternate| statement_requires_runtime_env(alternate))
        }
        Statement::While(statement) => {
            expression_requires_runtime_env(&statement.test)
                || statement_requires_runtime_env(&statement.body)
        }
        Statement::DoWhile(statement) => {
            statement_requires_runtime_env(&statement.body)
                || expression_requires_runtime_env(&statement.test)
        }
        Statement::For(statement) => for_statement_requires_runtime_env(statement),
        Statement::Return(statement) => statement
            .argument
            .as_ref()
            .is_some_and(expression_requires_runtime_env),
        Statement::Break(_) | Statement::Continue(_) => false,
        Statement::Expression(statement) => expression_requires_runtime_env(&statement.expression),
        Statement::Switch(statement) => {
            expression_requires_runtime_env(&statement.discriminant)
                || statement.cases.iter().any(|case| {
                    case.test
                        .as_ref()
                        .is_some_and(expression_requires_runtime_env)
                        || case.consequent.iter().any(statement_requires_runtime_env)
                })
        }
        Statement::Throw(statement) => expression_requires_runtime_env(&statement.argument),
        Statement::Try(statement) => {
            statement
                .block
                .body
                .iter()
                .any(statement_requires_runtime_env)
                || statement.handler.as_ref().is_some_and(|handler| {
                    handler
                        .param
                        .as_ref()
                        .is_some_and(pattern_requires_runtime_env)
                        || handler.body.body.iter().any(statement_requires_runtime_env)
                })
                || statement
                    .finalizer
                    .as_ref()
                    .is_some_and(|block| block.body.iter().any(statement_requires_runtime_env))
        }
        Statement::Labeled(_)
        | Statement::ImportDeclaration(_)
        | Statement::ExportDeclaration(_)
        | Statement::With(_) => true,
    }
}

fn for_statement_requires_runtime_env(statement: &ForStatement) -> bool {
    match statement {
        ForStatement::Classic(statement) => {
            statement.init.as_ref().is_some_and(|init| match init {
                ForInit::VariableDeclaration(declaration) => declaration
                    .declarations
                    .iter()
                    .any(variable_declarator_requires_runtime_env),
                ForInit::Expression(expression) => expression_requires_runtime_env(expression),
            }) || statement
                .test
                .as_ref()
                .is_some_and(expression_requires_runtime_env)
                || statement
                    .update
                    .as_ref()
                    .is_some_and(expression_requires_runtime_env)
                || statement_requires_runtime_env(&statement.body)
        }
        ForStatement::In(statement) => {
            for_left_requires_runtime_env(&statement.left)
                || expression_requires_runtime_env(&statement.right)
                || statement_requires_runtime_env(&statement.body)
        }
        ForStatement::Of(statement) => {
            for_left_requires_runtime_env(&statement.left)
                || expression_requires_runtime_env(&statement.right)
                || statement_requires_runtime_env(&statement.body)
        }
    }
}

fn for_left_requires_runtime_env(left: &ast::ForLeft) -> bool {
    match left {
        ast::ForLeft::VariableDeclaration(declaration) => declaration
            .declarations
            .iter()
            .any(variable_declarator_requires_runtime_env),
        ast::ForLeft::Pattern(pattern) => pattern_requires_runtime_env(pattern),
        ast::ForLeft::Expression(expression) => expression_requires_runtime_env(expression),
    }
}

fn variable_declarator_requires_runtime_env(declarator: &VariableDeclarator) -> bool {
    pattern_requires_runtime_env(&declarator.pattern)
        || declarator
            .init
            .as_ref()
            .is_some_and(expression_requires_runtime_env)
}

fn expression_requires_runtime_env(expression: &Expression) -> bool {
    match expression {
        Expression::Identifier(_) | Expression::Literal(_) | Expression::This(_) => false,
        Expression::Array(array) => array
            .elements
            .iter()
            .flatten()
            .any(|element| match element {
                ast::ArrayElement::Expression(expression) => {
                    expression_requires_runtime_env(expression)
                }
                ast::ArrayElement::Spread { argument, .. } => {
                    expression_requires_runtime_env(argument)
                }
            }),
        Expression::Object(object) => object
            .properties
            .iter()
            .any(object_property_requires_runtime_env),
        Expression::Function(_) | Expression::ArrowFunction(_) | Expression::Class(_) => true,
        Expression::Unary(unary) => expression_requires_runtime_env(&unary.argument),
        Expression::Update(update) => expression_requires_runtime_env(&update.argument),
        Expression::Binary(binary) => {
            expression_requires_runtime_env(&binary.left)
                || expression_requires_runtime_env(&binary.right)
        }
        Expression::Logical(binary) => {
            expression_requires_runtime_env(&binary.left)
                || expression_requires_runtime_env(&binary.right)
        }
        Expression::Assignment(assignment) => {
            expression_requires_runtime_env(&assignment.left)
                || expression_requires_runtime_env(&assignment.right)
        }
        Expression::Conditional(conditional) => {
            expression_requires_runtime_env(&conditional.test)
                || expression_requires_runtime_env(&conditional.consequent)
                || expression_requires_runtime_env(&conditional.alternate)
        }
        Expression::Sequence(sequence) => sequence
            .expressions
            .iter()
            .any(expression_requires_runtime_env),
        Expression::Call(call) => {
            expression_requires_runtime_env(&call.callee)
                || call.arguments.iter().any(|argument| match argument {
                    CallArgument::Expression(expression) => {
                        expression_requires_runtime_env(expression)
                    }
                    CallArgument::Spread { argument, .. } => {
                        expression_requires_runtime_env(argument)
                    }
                })
        }
        Expression::New(new_expr) => {
            expression_requires_runtime_env(&new_expr.callee)
                || new_expr.arguments.iter().any(|argument| match argument {
                    CallArgument::Expression(expression) => {
                        expression_requires_runtime_env(expression)
                    }
                    CallArgument::Spread { argument, .. } => {
                        expression_requires_runtime_env(argument)
                    }
                })
        }
        Expression::Member(member) => {
            expression_requires_runtime_env(&member.object)
                || match &member.property {
                    MemberProperty::Identifier(_) | MemberProperty::PrivateName(_) => false,
                    MemberProperty::Computed { expression, .. } => {
                        expression_requires_runtime_env(expression)
                    }
                }
        }
        _ => true,
    }
}

fn object_property_requires_runtime_env(property: &ObjectProperty) -> bool {
    match property {
        ObjectProperty::Property {
            key,
            value,
            shorthand: _,
            kind: _,
            span: _,
        } => property_key_requires_runtime_env(key) || expression_requires_runtime_env(value),
        ObjectProperty::Spread { argument, .. } => expression_requires_runtime_env(argument),
    }
}

fn property_key_requires_runtime_env(key: &PropertyKey) -> bool {
    match key {
        PropertyKey::Identifier(_)
        | PropertyKey::PrivateName(_)
        | PropertyKey::String(_)
        | PropertyKey::Number(_) => false,
        PropertyKey::Computed { expression, .. } => expression_requires_runtime_env(expression),
    }
}

#[derive(Debug)]
enum TemplatePart {
    Text(String),
    Expression(Expression),
}

fn parse_template_literal_parts(
    literal: &ast::TemplateLiteral,
) -> Result<Vec<TemplatePart>, CodegenError> {
    let value = literal.value.as_str();
    let mut parts = Vec::new();
    let mut text_start = 0usize;
    let mut search_start = 0usize;

    while let Some(relative_start) = value[search_start..].find("${") {
        let expr_start = search_start + relative_start;
        if expr_start > text_start {
            parts.push(TemplatePart::Text(value[text_start..expr_start].to_owned()));
        }

        let body_start = expr_start + 2;
        let mut matched = None;
        for (relative_end, ch) in value[body_start..].char_indices() {
            if ch != '}' {
                continue;
            }

            let expression_source = &value[body_start..body_start + relative_end];
            match parse_template_expression(expression_source, literal.span) {
                Ok(expression) => {
                    matched = Some((body_start + relative_end + ch.len_utf8(), expression));
                    break;
                }
                Err(CodegenError::Parse(_)) => continue,
                Err(error) => return Err(error),
            }
        }

        let Some((next_index, expression)) = matched else {
            return Err(CodegenError::Unsupported {
                feature: "template literal interpolations",
                span: literal.span,
            });
        };

        parts.push(TemplatePart::Expression(expression));
        text_start = next_index;
        search_start = next_index;
    }

    if text_start < value.len() {
        parts.push(TemplatePart::Text(value[text_start..].to_owned()));
    }

    Ok(parts)
}

fn parse_template_expression(source: &str, span: Span) -> Result<Expression, CodegenError> {
    let wrapped = format!("({source});");
    let program = ast::parse(&wrapped)?;
    let mut statements = program.body.into_iter();
    match statements.next() {
        Some(Statement::Expression(ExpressionStatement { expression, .. }))
            if statements.next().is_none() =>
        {
            Ok(expression)
        }
        _ => Err(CodegenError::Unsupported {
            feature: "template literal interpolations",
            span,
        }),
    }
}

fn offset_to(target: usize, current_len: usize) -> Result<i16, CodegenError> {
    i16::try_from(target as isize - current_len as isize - 1).map_err(|_| {
        CodegenError::Unsupported {
            feature: "jump offset out of range",
            span: Span::default(),
        }
    })
}

fn encode_asbx(opcode: Opcode, a: u8, sbx: i16) -> u32 {
    (((sbx as u16) as u32) << 16) | ((a as u32) << 8) | opcode.as_u8() as u32
}
