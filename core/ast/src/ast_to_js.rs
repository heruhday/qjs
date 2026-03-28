use crate::ast::*;

/// Converts the parser AST back into JavaScript source.
///
/// The emitter is deterministic and favors safe reparsing over preserving the
/// original surface form.
pub fn program_to_js(program: &Program) -> String {
    JsEmitter.emit_program(program)
}

pub fn statement_to_js(statement: &Statement) -> String {
    JsEmitter.emit_statement(statement, 0)
}

pub fn expression_to_js(expression: &Expression) -> String {
    JsEmitter.emit_expression(expression, PREC_LOWEST)
}

#[derive(Clone, Copy)]
struct JsEmitter;

const PREC_LOWEST: u8 = 0;
const PREC_SEQUENCE: u8 = 1;
const PREC_ASSIGNMENT: u8 = 2;
const PREC_YIELD: u8 = 3;
const PREC_CONDITIONAL: u8 = 4;
const PREC_LOGICAL_OR: u8 = 5;
const PREC_NULLISH: u8 = 6;
const PREC_LOGICAL_AND: u8 = 7;
const PREC_BIT_OR: u8 = 8;
const PREC_BIT_XOR: u8 = 9;
const PREC_BIT_AND: u8 = 10;
const PREC_EQUALITY: u8 = 11;
const PREC_RELATIONAL: u8 = 12;
const PREC_SHIFT: u8 = 13;
const PREC_ADDITIVE: u8 = 14;
const PREC_MULTIPLICATIVE: u8 = 15;
const PREC_EXPONENTIATION: u8 = 16;
const PREC_UNARY: u8 = 17;
const PREC_UPDATE: u8 = 18;
const PREC_NEW: u8 = 19;
const PREC_LEFT_HAND_SIDE: u8 = 20;
const PREC_PRIMARY: u8 = 21;

impl JsEmitter {
    fn emit_program(self, program: &Program) -> String {
        self.emit_statement_list(&program.body, 0)
    }

    fn emit_statement(self, statement: &Statement, indent: usize) -> String {
        let pad = self.indent(indent);
        match statement {
            Statement::Directive(directive) => {
                format!("{pad}{};", self.emit_string_literal(&directive.value))
            }
            Statement::Empty(_) => format!("{pad};"),
            Statement::Debugger(_) => format!("{pad}debugger;"),
            Statement::Block(block) => format!("{pad}{}", self.emit_block(block, indent)),
            Statement::Labeled(statement) => {
                let body = match statement.body.as_ref() {
                    Statement::Block(block) => format!(" {}", self.emit_block(block, indent)),
                    _ => format!(
                        "\n{}",
                        self.emit_statement_in_statement_position(
                            statement.body.as_ref(),
                            indent + 1,
                        )
                    ),
                };
                format!("{pad}{}:{body}", self.emit_identifier(&statement.label))
            }
            Statement::ImportDeclaration(declaration) => {
                format!("{pad}{};", self.emit_import_declaration(declaration))
            }
            Statement::ExportDeclaration(declaration) => {
                format!("{pad}{}", self.emit_export_declaration(declaration, indent))
            }
            Statement::VariableDeclaration(declaration) => {
                format!("{pad}{};", self.emit_variable_declaration(declaration))
            }
            Statement::FunctionDeclaration(function) => {
                format!("{pad}{}", self.emit_function(function, true, indent))
            }
            Statement::ClassDeclaration(class) => self.emit_class_statement(class, indent),
            Statement::If(statement) => self.emit_if_statement(statement, indent),
            Statement::While(statement) => format!(
                "{pad}while ({}){}",
                self.emit_expression(&statement.test, PREC_LOWEST),
                self.emit_embedded_statement(statement.body.as_ref(), indent)
            ),
            Statement::DoWhile(statement) => {
                let body = self.emit_embedded_statement(statement.body.as_ref(), indent);
                if matches!(statement.body.as_ref(), Statement::Block(_)) {
                    format!(
                        "{pad}do{body} while ({});",
                        self.emit_expression(&statement.test, PREC_LOWEST)
                    )
                } else {
                    format!(
                        "{pad}do{body}\n{pad}while ({});",
                        self.emit_expression(&statement.test, PREC_LOWEST)
                    )
                }
            }
            Statement::For(statement) => self.emit_for_statement(statement, indent),
            Statement::Switch(statement) => self.emit_switch_statement(statement, indent),
            Statement::Return(statement) => match &statement.argument {
                Some(argument) => {
                    format!(
                        "{pad}return {};",
                        self.emit_expression(argument, PREC_LOWEST)
                    )
                }
                None => format!("{pad}return;"),
            },
            Statement::Break(statement) => match &statement.label {
                Some(label) => format!("{pad}break {};", self.emit_identifier(label)),
                None => format!("{pad}break;"),
            },
            Statement::Continue(statement) => match &statement.label {
                Some(label) => format!("{pad}continue {};", self.emit_identifier(label)),
                None => format!("{pad}continue;"),
            },
            Statement::Throw(statement) => {
                format!(
                    "{pad}throw {};",
                    self.emit_expression(&statement.argument, PREC_LOWEST)
                )
            }
            Statement::Try(statement) => self.emit_try_statement(statement, indent),
            Statement::With(statement) => format!(
                "{pad}with ({}){}",
                self.emit_expression(&statement.object, PREC_LOWEST),
                self.emit_embedded_statement(statement.body.as_ref(), indent)
            ),
            Statement::Expression(statement) => {
                self.emit_expression_statement(&statement.expression, indent)
            }
        }
    }

    fn emit_expression_statement(self, expression: &Expression, indent: usize) -> String {
        let pad = self.indent(indent);
        let emitted = self.emit_expression(expression, PREC_LOWEST);
        if self.expression_statement_requires_parentheses(expression) {
            format!("{pad}({emitted});")
        } else {
            format!("{pad}{emitted};")
        }
    }

    fn emit_if_statement(self, statement: &IfStatement, indent: usize) -> String {
        let pad = self.indent(indent);
        let mut out = format!(
            "{pad}if ({}){}",
            self.emit_expression(&statement.test, PREC_LOWEST),
            self.emit_embedded_statement(statement.consequent.as_ref(), indent)
        );
        if let Some(alternate) = &statement.alternate {
            if matches!(statement.consequent.as_ref(), Statement::Block(_)) {
                out.push(' ');
            } else {
                out.push('\n');
                out.push_str(&pad);
            }
            out.push_str("else");
            out.push_str(&self.emit_embedded_statement(alternate.as_ref(), indent));
        }
        out
    }

    fn emit_for_statement(self, statement: &ForStatement, indent: usize) -> String {
        let pad = self.indent(indent);
        match statement {
            ForStatement::Classic(statement) => {
                let init = statement
                    .init
                    .as_ref()
                    .map(|init| self.emit_for_init(init))
                    .unwrap_or_default();
                let test = statement
                    .test
                    .as_ref()
                    .map(|test| self.emit_expression(test, PREC_LOWEST))
                    .unwrap_or_default();
                let update = statement
                    .update
                    .as_ref()
                    .map(|update| self.emit_expression(update, PREC_LOWEST))
                    .unwrap_or_default();
                format!(
                    "{pad}for ({init}; {test}; {update}){}",
                    self.emit_embedded_statement(statement.body.as_ref(), indent)
                )
            }
            ForStatement::In(statement) => format!(
                "{pad}for ({} in {}){}",
                self.emit_for_left(&statement.left),
                self.emit_expression(&statement.right, PREC_LOWEST),
                self.emit_embedded_statement(statement.body.as_ref(), indent)
            ),
            ForStatement::Of(statement) => {
                let await_prefix = if statement.is_await { " await" } else { "" };
                let right = match &statement.right {
                    Expression::Sequence(_) => {
                        format!("({})", self.emit_expression(&statement.right, PREC_LOWEST))
                    }
                    _ => self.emit_expression(&statement.right, PREC_LOWEST),
                };
                format!(
                    "{pad}for{await_prefix} ({} of {}){}",
                    self.emit_for_left(&statement.left),
                    right,
                    self.emit_embedded_statement(statement.body.as_ref(), indent)
                )
            }
        }
    }

    fn emit_switch_statement(self, statement: &SwitchStatement, indent: usize) -> String {
        let pad = self.indent(indent);
        if statement.cases.is_empty() {
            return format!(
                "{pad}switch ({}) {{}}",
                self.emit_expression(&statement.discriminant, PREC_LOWEST)
            );
        }

        let mut out = format!(
            "{pad}switch ({}) {{\n",
            self.emit_expression(&statement.discriminant, PREC_LOWEST)
        );
        for (index, case) in statement.cases.iter().enumerate() {
            out.push_str(&self.emit_switch_case(case, indent + 1));
            if index + 1 < statement.cases.len() {
                out.push('\n');
            }
        }
        out.push('\n');
        out.push_str(&pad);
        out.push('}');
        out
    }

    fn emit_switch_case(self, case: &SwitchCase, indent: usize) -> String {
        let pad = self.indent(indent);
        let mut out = match &case.test {
            Some(test) => format!("{pad}case {}:", self.emit_expression(test, PREC_LOWEST)),
            None => format!("{pad}default:"),
        };
        if !case.consequent.is_empty() {
            out.push('\n');
            out.push_str(
                &case
                    .consequent
                    .iter()
                    .map(|statement| self.emit_statement(statement, indent + 1))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        out
    }

    fn emit_try_statement(self, statement: &TryStatement, indent: usize) -> String {
        let pad = self.indent(indent);
        let mut out = format!("{pad}try {}", self.emit_block(&statement.block, indent));
        if let Some(handler) = &statement.handler {
            out.push_str(" catch");
            if let Some(param) = &handler.param {
                out.push_str(" (");
                out.push_str(&self.emit_pattern(param));
                out.push(')');
            }
            out.push(' ');
            out.push_str(&self.emit_block(&handler.body, indent));
        }
        if let Some(finalizer) = &statement.finalizer {
            out.push_str(" finally ");
            out.push_str(&self.emit_block(finalizer, indent));
        }
        out
    }

    fn emit_import_declaration(self, declaration: &ImportDeclaration) -> String {
        let mut out = String::from("import ");
        match &declaration.clause {
            None => {
                out.push_str(&self.emit_string_literal_node(&declaration.source));
            }
            Some(clause) => {
                if declaration.is_defer {
                    out.push_str("defer ");
                }
                out.push_str(&self.emit_import_clause(clause));
                out.push_str(" from ");
                out.push_str(&self.emit_string_literal_node(&declaration.source));
            }
        }
        out.push_str(&self.emit_module_attributes(declaration.attributes.as_ref()));
        out
    }

    fn emit_import_clause(self, clause: &ImportClause) -> String {
        match clause {
            ImportClause::Default(identifier) => self.emit_identifier(identifier),
            ImportClause::Namespace { default, namespace } => {
                let mut parts = Vec::new();
                if let Some(default) = default {
                    parts.push(self.emit_identifier(default));
                }
                parts.push(format!("* as {}", self.emit_identifier(namespace)));
                parts.join(", ")
            }
            ImportClause::Named {
                default,
                specifiers,
            } => {
                let mut parts = Vec::new();
                if let Some(default) = default {
                    parts.push(self.emit_identifier(default));
                }
                let specifiers = specifiers
                    .iter()
                    .map(|specifier| self.emit_import_specifier(specifier))
                    .collect::<Vec<_>>()
                    .join(", ");
                parts.push(format!("{{ {specifiers} }}"));
                parts.join(", ")
            }
        }
    }

    fn emit_import_specifier(self, specifier: &ImportSpecifier) -> String {
        let imported = self.emit_module_export_name(&specifier.imported);
        let local = self.emit_identifier(&specifier.local);
        if imported == local {
            imported
        } else {
            format!("{imported} as {local}")
        }
    }

    fn emit_export_declaration(self, declaration: &ExportDeclaration, indent: usize) -> String {
        match declaration {
            ExportDeclaration::All(declaration) => {
                let mut out = String::from("export *");
                if let Some(exported) = &declaration.exported {
                    out.push_str(" as ");
                    out.push_str(&self.emit_module_export_name(exported));
                }
                out.push_str(" from ");
                out.push_str(&self.emit_string_literal_node(&declaration.source));
                out.push_str(&self.emit_module_attributes(declaration.attributes.as_ref()));
                out.push(';');
                out
            }
            ExportDeclaration::Named(declaration) => {
                let specifiers = declaration
                    .specifiers
                    .iter()
                    .map(|specifier| self.emit_export_specifier(specifier))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut out = format!("export {{ {specifiers} }}");
                if let Some(source) = &declaration.source {
                    out.push_str(" from ");
                    out.push_str(&self.emit_string_literal_node(source));
                }
                out.push_str(&self.emit_module_attributes(declaration.attributes.as_ref()));
                out.push(';');
                out
            }
            ExportDeclaration::Default(declaration) => match &declaration.declaration {
                ExportDefaultKind::Function(function) => {
                    format!(
                        "export default {}",
                        self.emit_function(function, true, indent)
                    )
                }
                ExportDefaultKind::Class(class) => {
                    format!("export default {}", self.emit_class(class, indent))
                }
                ExportDefaultKind::Expression(expression) => {
                    let emitted =
                        if matches!(expression, Expression::Function(_) | Expression::Class(_)) {
                            format!("({})", self.emit_expression(expression, PREC_LOWEST))
                        } else {
                            self.emit_expression(expression, PREC_LOWEST)
                        };
                    format!("export default {};", emitted)
                }
            },
            ExportDeclaration::Declaration(declaration) => match declaration {
                ExportedDeclaration::Variable(declaration) => {
                    format!("export {};", self.emit_variable_declaration(declaration))
                }
                ExportedDeclaration::Function(function) => {
                    format!("export {}", self.emit_function(function, true, indent))
                }
                ExportedDeclaration::Class(class) => {
                    format!("export {}", self.emit_class(class, indent))
                }
            },
        }
    }

    fn emit_export_specifier(self, specifier: &ExportSpecifier) -> String {
        let local = self.emit_module_export_name(&specifier.local);
        let exported = self.emit_module_export_name(&specifier.exported);
        if local == exported {
            local
        } else {
            format!("{local} as {exported}")
        }
    }

    fn emit_module_export_name(self, name: &ModuleExportName) -> String {
        match name {
            ModuleExportName::Identifier(identifier) => self.emit_identifier(identifier),
            ModuleExportName::String(string) => self.emit_string_literal_node(string),
        }
    }

    fn emit_module_attributes(self, attributes: Option<&Expression>) -> String {
        attributes
            .map(|attributes| format!(" with {}", self.emit_expression(attributes, PREC_LOWEST)))
            .unwrap_or_default()
    }

    fn emit_variable_declaration(self, declaration: &VariableDeclaration) -> String {
        let declarations = declaration
            .declarations
            .iter()
            .map(|declarator| self.emit_variable_declarator(declarator))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} {declarations}",
            self.emit_variable_kind(declaration.kind)
        )
    }

    fn emit_variable_declarator(self, declarator: &VariableDeclarator) -> String {
        let mut out = self.emit_pattern(&declarator.pattern);
        if let Some(init) = &declarator.init {
            out.push_str(" = ");
            out.push_str(&self.emit_variable_initializer(init));
        }
        out
    }

    fn emit_variable_kind(self, kind: VariableKind) -> &'static str {
        match kind {
            VariableKind::Var => "var",
            VariableKind::Let => "let",
            VariableKind::Const => "const",
            VariableKind::Using => "using",
            VariableKind::AwaitUsing => "await using",
        }
    }

    fn emit_function(self, function: &Function, emit_name: bool, indent: usize) -> String {
        let mut out = String::new();
        if function.is_async {
            out.push_str("async ");
        }
        out.push_str("function");
        if function.is_generator {
            out.push('*');
        }
        if emit_name {
            if let Some(id) = &function.id {
                out.push(' ');
                out.push_str(&self.emit_identifier(id));
            }
        }
        out.push('(');
        out.push_str(
            &function
                .params
                .iter()
                .map(|pattern| self.emit_pattern(pattern))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(") ");
        out.push_str(&self.emit_block(&function.body, indent));
        out
    }

    fn emit_class_statement(self, class: &Class, indent: usize) -> String {
        let pad = self.indent(indent);
        let decorators = self.emit_decorators(&class.decorators, indent);
        if decorators.is_empty() {
            format!("{pad}{}", self.emit_class(class, indent))
        } else {
            format!("{decorators}\n{pad}{}", self.emit_class(class, indent))
        }
    }

    fn emit_class(self, class: &Class, indent: usize) -> String {
        let mut out = String::from("class");
        if let Some(id) = &class.id {
            out.push(' ');
            out.push_str(&self.emit_identifier(id));
        }
        if let Some(super_class) = &class.super_class {
            out.push_str(" extends ");
            out.push_str(&self.emit_expression(super_class, PREC_RELATIONAL));
        }
        out.push(' ');
        out.push_str(&self.emit_class_body(&class.body, indent));
        out
    }

    fn emit_class_body(self, body: &[ClassElement], indent: usize) -> String {
        if body.is_empty() {
            return "{}".to_string();
        }

        let mut out = String::from("{\n");
        for (index, element) in body.iter().enumerate() {
            out.push_str(&self.emit_class_element(element, indent + 1));
            if index + 1 < body.len() {
                out.push('\n');
            }
        }
        out.push('\n');
        out.push_str(&self.indent(indent));
        out.push('}');
        out
    }

    fn emit_class_element(self, element: &ClassElement, indent: usize) -> String {
        match element {
            ClassElement::Empty(_) => format!("{};", self.indent(indent)),
            ClassElement::StaticBlock(block) => {
                format!(
                    "{}static {}",
                    self.indent(indent),
                    self.emit_block(block, indent)
                )
            }
            ClassElement::Method(method) => self.emit_class_method(method, indent),
            ClassElement::Field(field) => self.emit_class_field(field, indent),
        }
    }

    fn emit_class_method(self, method: &ClassMethod, indent: usize) -> String {
        let decorators = self.emit_decorators(&method.decorators, indent);
        let mut signature = String::new();
        if method.is_static {
            signature.push_str("static ");
        }
        match method.kind {
            MethodKind::Getter => signature.push_str("get "),
            MethodKind::Setter => signature.push_str("set "),
            MethodKind::Method | MethodKind::Constructor => {}
        }
        if method.value.is_async {
            signature.push_str("async ");
        }
        if method.value.is_generator {
            signature.push('*');
        }
        signature.push_str(&self.emit_property_key(&method.key));
        signature.push('(');
        signature.push_str(
            &method
                .value
                .params
                .iter()
                .map(|pattern| self.emit_pattern(pattern))
                .collect::<Vec<_>>()
                .join(", "),
        );
        signature.push_str(") ");
        signature.push_str(&self.emit_block(&method.value.body, indent));

        let line = format!("{}{}", self.indent(indent), signature);
        if decorators.is_empty() {
            line
        } else {
            format!("{decorators}\n{line}")
        }
    }

    fn emit_class_field(self, field: &ClassField, indent: usize) -> String {
        let decorators = self.emit_decorators(&field.decorators, indent);
        let mut line = String::new();
        line.push_str(&self.indent(indent));
        if field.is_static {
            line.push_str("static ");
        }
        if field.is_accessor {
            line.push_str("accessor ");
        }
        line.push_str(&self.emit_property_key(&field.key));
        if let Some(value) = &field.value {
            line.push_str(" = ");
            line.push_str(&self.emit_field_initializer(value));
        }
        line.push(';');
        if decorators.is_empty() {
            line
        } else {
            format!("{decorators}\n{line}")
        }
    }

    fn emit_decorators(self, decorators: &[Expression], indent: usize) -> String {
        let pad = self.indent(indent);
        decorators
            .iter()
            .map(|decorator| format!("{pad}@{}", self.emit_expression(decorator, PREC_PRIMARY)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn emit_pattern(self, pattern: &Pattern) -> String {
        match pattern {
            Pattern::Identifier(identifier) => self.emit_identifier(identifier),
            Pattern::Array(pattern) => {
                let mut out = String::from("[");
                for (index, element) in pattern.elements.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    if let Some(element) = element {
                        out.push_str(&self.emit_pattern(element));
                    }
                }
                if matches!(pattern.elements.last(), Some(None)) {
                    out.push(',');
                }
                out.push(']');
                out
            }
            Pattern::Object(pattern) => {
                let properties = pattern
                    .properties
                    .iter()
                    .map(|property| self.emit_object_pattern_property(property))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {properties} }}")
            }
            Pattern::Rest(pattern) => {
                format!("...{}", self.emit_pattern(pattern.argument.as_ref()))
            }
            Pattern::Assignment(pattern) => format!(
                "{} = {}",
                self.emit_pattern(pattern.left.as_ref()),
                self.emit_expression(&pattern.right, PREC_ASSIGNMENT)
            ),
        }
    }

    fn emit_object_pattern_property(self, property: &ObjectPatternProperty) -> String {
        match property {
            ObjectPatternProperty::Property {
                key,
                value,
                shorthand,
                ..
            } => {
                if *shorthand {
                    self.emit_pattern(value.as_ref())
                } else {
                    format!(
                        "{}: {}",
                        self.emit_property_key(key),
                        self.emit_pattern(value.as_ref())
                    )
                }
            }
            ObjectPatternProperty::Rest { argument, .. } => {
                format!("...{}", self.emit_pattern(argument.as_ref()))
            }
        }
    }

    fn emit_expression(self, expression: &Expression, min_prec: u8) -> String {
        let prec = self.expression_precedence(expression);
        let body = match expression {
            Expression::Identifier(identifier) => self.emit_identifier(identifier),
            Expression::PrivateIdentifier(identifier) => format!("#{}", identifier.name),
            Expression::Literal(literal) => self.emit_literal(literal),
            Expression::This(_) => "this".to_string(),
            Expression::Super(_) => "super".to_string(),
            Expression::Array(array) => self.emit_array_expression(array),
            Expression::Object(object) => self.emit_object_expression(object),
            Expression::Function(function) => {
                self.emit_function(function, function.id.is_some(), 0)
            }
            Expression::ArrowFunction(function) => self.emit_arrow_function(function),
            Expression::Class(class) => {
                let decorators = self.emit_decorators(&class.decorators, 0);
                if decorators.is_empty() {
                    self.emit_class(class, 0)
                } else {
                    format!("{decorators}\n{}", self.emit_class(class, 0))
                }
            }
            Expression::TaggedTemplate(expression) => format!(
                "{}{}",
                self.emit_chain_base(&expression.tag),
                self.emit_template_literal_node(&expression.quasi)
            ),
            Expression::MetaProperty(expression) => format!(
                "{}.{}",
                self.emit_identifier(&expression.meta),
                self.emit_identifier(&expression.property)
            ),
            Expression::Yield(expression) => {
                let mut out = String::from("yield");
                if expression.delegate {
                    out.push('*');
                }
                if let Some(argument) = &expression.argument {
                    out.push(' ');
                    out.push_str(&self.emit_expression(argument, PREC_ASSIGNMENT));
                }
                out
            }
            Expression::Await(expression) => {
                format!(
                    "await {}",
                    self.emit_expression(&expression.argument, PREC_UNARY)
                )
            }
            Expression::Unary(expression) => {
                let operator = self.emit_unary_operator(expression.operator);
                let argument = if matches!(&expression.argument, Expression::Binary(binary) if binary.operator == BinaryOperator::Exponentiate)
                    || matches!(
                        (expression.operator, &expression.argument),
                        (
                            UnaryOperator::Positive | UnaryOperator::Negative,
                            Expression::Unary(_) | Expression::Update(_)
                        )
                    ) {
                    format!(
                        "({})",
                        self.emit_expression(&expression.argument, PREC_LOWEST)
                    )
                } else {
                    self.emit_expression(&expression.argument, PREC_UNARY)
                };
                format!("{operator}{argument}")
            }
            Expression::Update(expression) => {
                let argument = self.emit_expression(&expression.argument, PREC_UPDATE);
                let operator = self.emit_update_operator(expression.operator);
                if expression.prefix {
                    format!("{operator}{argument}")
                } else {
                    format!("{argument}{operator}")
                }
            }
            Expression::Binary(expression) => self.emit_binary_expression(expression),
            Expression::Logical(expression) => self.emit_logical_expression(expression),
            Expression::Assignment(expression) => self.emit_assignment_expression(expression),
            Expression::Conditional(expression) => self.emit_conditional_expression(expression),
            Expression::Sequence(expression) => expression
                .expressions
                .iter()
                .map(|expression| self.emit_expression(expression, PREC_ASSIGNMENT))
                .collect::<Vec<_>>()
                .join(", "),
            Expression::Call(expression) => self.emit_call_expression(expression),
            Expression::Member(expression) => self.emit_member_expression(expression),
            Expression::New(expression) => self.emit_new_expression(expression),
        };

        if prec < min_prec {
            format!("({body})")
        } else {
            body
        }
    }

    fn emit_array_expression(self, array: &ArrayExpression) -> String {
        let mut out = String::from("[");
        for (index, element) in array.elements.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            match element {
                Some(ArrayElement::Expression(expression)) => {
                    out.push_str(&self.emit_expression(expression, PREC_ASSIGNMENT));
                }
                Some(ArrayElement::Spread { argument, .. }) => {
                    out.push_str("...");
                    out.push_str(&self.emit_expression(argument, PREC_ASSIGNMENT));
                }
                None => {}
            }
        }
        if matches!(array.elements.last(), Some(None)) {
            out.push(',');
        }
        out.push(']');
        out
    }

    fn emit_object_expression(self, object: &ObjectExpression) -> String {
        let properties = object
            .properties
            .iter()
            .map(|property| self.emit_object_property(property))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{ {properties} }}")
    }

    fn emit_object_property(self, property: &ObjectProperty) -> String {
        match property {
            ObjectProperty::Spread { argument, .. } => {
                format!("...{}", self.emit_expression(argument, PREC_ASSIGNMENT))
            }
            ObjectProperty::Property {
                key,
                value,
                shorthand,
                kind,
                ..
            } => {
                if *kind != ObjectPropertyKind::Init {
                    if let Expression::Function(function) = value {
                        let mut out = String::new();
                        match kind {
                            ObjectPropertyKind::Getter => out.push_str("get "),
                            ObjectPropertyKind::Setter => out.push_str("set "),
                            ObjectPropertyKind::Method | ObjectPropertyKind::Init => {}
                        }
                        if function.is_async {
                            out.push_str("async ");
                        }
                        if function.is_generator {
                            out.push('*');
                        }
                        out.push_str(&self.emit_property_key(key));
                        out.push('(');
                        out.push_str(
                            &function
                                .params
                                .iter()
                                .map(|pattern| self.emit_pattern(pattern))
                                .collect::<Vec<_>>()
                                .join(", "),
                        );
                        out.push_str(") ");
                        out.push_str(&self.emit_block(&function.body, 0));
                        return out;
                    }
                }

                if *shorthand {
                    self.emit_expression(value, PREC_ASSIGNMENT)
                } else {
                    format!(
                        "{}: {}",
                        self.emit_property_key(key),
                        self.emit_expression(value, PREC_ASSIGNMENT)
                    )
                }
            }
        }
    }

    fn emit_arrow_function(self, function: &ArrowFunction) -> String {
        let params = function
            .params
            .iter()
            .map(|pattern| self.emit_pattern(pattern))
            .collect::<Vec<_>>()
            .join(", ");
        let body = match &function.body {
            ArrowBody::Expression(expression) => {
                if matches!(
                    expression.as_ref(),
                    Expression::Object(_) | Expression::Sequence(_)
                ) {
                    format!(
                        "({})",
                        self.emit_expression(expression.as_ref(), PREC_LOWEST)
                    )
                } else {
                    self.emit_expression(expression.as_ref(), PREC_ASSIGNMENT)
                }
            }
            ArrowBody::Block(block) => self.emit_block(block, 0),
        };
        if function.is_async {
            format!("async ({params}) => {body}")
        } else {
            format!("({params}) => {body}")
        }
    }

    fn emit_literal(self, literal: &Literal) -> String {
        match literal {
            Literal::Null(_) => "null".to_string(),
            Literal::Boolean(literal) => {
                if literal.value {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            Literal::Number(literal) => literal.raw.clone(),
            Literal::String(literal) => self.emit_string_literal_node(literal),
            Literal::Template(literal) => self.emit_template_literal_node(literal),
            Literal::RegExp(literal) => format!("/{}/{}", literal.body, literal.flags),
        }
    }

    fn emit_string_literal_node(self, literal: &StringLiteral) -> String {
        self.emit_string_literal(&literal.value)
    }

    fn emit_string_literal(self, value: &str) -> String {
        let mut out = String::from("\"");
        for ch in value.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\u{08}' => out.push_str("\\b"),
                '\u{0C}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{0B}' => out.push_str("\\v"),
                '\u{2028}' => out.push_str("\\u2028"),
                '\u{2029}' => out.push_str("\\u2029"),
                ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
                ch => out.push(ch),
            }
        }
        out.push('"');
        out
    }

    fn emit_template_literal_node(self, literal: &TemplateLiteral) -> String {
        let mut out = String::from("`");
        let mut chars = literal.value.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '`' => out.push_str("\\`"),
                '\\' => out.push_str("\\\\"),
                '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0C}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{0B}' => out.push_str("\\v"),
                '\u{2028}' => out.push_str("\\u2028"),
                '\u{2029}' => out.push_str("\\u2029"),
                ch => out.push(ch),
            }
        }
        out.push('`');
        out
    }

    fn emit_binary_expression(self, expression: &BinaryExpression) -> String {
        let prec = self.expression_precedence(&Expression::Binary(Box::new(expression.clone())));
        let mut left = self.emit_expression(
            &expression.left,
            if expression.operator == BinaryOperator::Exponentiate {
                prec + 1
            } else {
                prec
            },
        );
        if expression.operator == BinaryOperator::Exponentiate
            && matches!(expression.left, Expression::Unary(_) | Expression::Await(_))
        {
            left = format!("({left})");
        }
        let right_prec = if expression.operator == BinaryOperator::Exponentiate {
            prec
        } else {
            prec + 1
        };
        format!(
            "{} {} {}",
            left,
            self.emit_binary_operator(expression.operator),
            self.emit_expression(&expression.right, right_prec)
        )
    }

    fn emit_logical_expression(self, expression: &LogicalExpression) -> String {
        let prec = self.expression_precedence(&Expression::Logical(Box::new(expression.clone())));
        let left = self.emit_logical_operand(&expression.left, expression.operator, prec);
        let right = self.emit_logical_operand(&expression.right, expression.operator, prec + 1);
        format!(
            "{} {} {}",
            left,
            self.emit_logical_operator(expression.operator),
            right
        )
    }

    fn emit_assignment_expression(self, expression: &AssignmentExpression) -> String {
        let prec =
            self.expression_precedence(&Expression::Assignment(Box::new(expression.clone())));
        format!(
            "{} {} {}",
            self.emit_expression(&expression.left, prec + 1),
            self.emit_assignment_operator(expression.operator),
            self.emit_expression(&expression.right, prec)
        )
    }

    fn emit_conditional_expression(self, expression: &ConditionalExpression) -> String {
        let prec =
            self.expression_precedence(&Expression::Conditional(Box::new(expression.clone())));
        format!(
            "{} ? {} : {}",
            self.emit_expression(&expression.test, prec + 1),
            self.emit_expression(&expression.consequent, PREC_ASSIGNMENT),
            self.emit_expression(&expression.alternate, prec)
        )
    }

    fn emit_call_expression(self, expression: &CallExpression) -> String {
        let mut out = self.emit_chain_base(&expression.callee);
        if expression.optional {
            out.push_str("?.");
        }
        out.push('(');
        out.push_str(
            &expression
                .arguments
                .iter()
                .map(|argument| self.emit_call_argument(argument))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push(')');
        out
    }

    fn emit_call_argument(self, argument: &CallArgument) -> String {
        match argument {
            CallArgument::Expression(expression) => {
                self.emit_expression(expression, PREC_ASSIGNMENT)
            }
            CallArgument::Spread { argument, .. } => {
                format!("...{}", self.emit_expression(argument, PREC_ASSIGNMENT))
            }
        }
    }

    fn emit_member_expression(self, expression: &MemberExpression) -> String {
        let mut out = self.emit_chain_base(&expression.object);
        match &expression.property {
            MemberProperty::Identifier(identifier) => {
                if expression.optional {
                    out.push_str("?.");
                } else {
                    out.push('.');
                }
                out.push_str(&self.emit_identifier(identifier));
            }
            MemberProperty::PrivateName(identifier) => {
                if expression.optional {
                    out.push_str("?.");
                } else {
                    out.push('.');
                }
                out.push('#');
                out.push_str(&identifier.name);
            }
            MemberProperty::Computed {
                expression: property,
                ..
            } => {
                if expression.optional {
                    out.push_str("?.");
                }
                out.push('[');
                out.push_str(&self.emit_expression(property.as_ref(), PREC_LOWEST));
                out.push(']');
            }
        }
        out
    }

    fn emit_new_expression(self, expression: &NewExpression) -> String {
        let mut out = String::from("new ");
        out.push_str(&self.emit_new_callee(&expression.callee));
        if !expression.arguments.is_empty() {
            out.push('(');
            out.push_str(
                &expression
                    .arguments
                    .iter()
                    .map(|argument| self.emit_call_argument(argument))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push(')');
        }
        out
    }

    fn emit_new_callee(self, expression: &Expression) -> String {
        if self.new_callee_requires_parentheses(expression) {
            format!("({})", self.emit_expression(expression, PREC_LOWEST))
        } else if self.expression_can_be_chain_base(expression) {
            self.emit_expression(expression, PREC_NEW)
        } else {
            format!("({})", self.emit_expression(expression, PREC_LOWEST))
        }
    }

    fn new_callee_requires_parentheses(self, expression: &Expression) -> bool {
        match expression {
            Expression::Call(_) | Expression::TaggedTemplate(_) => true,
            Expression::Member(member) => self.new_callee_requires_parentheses(&member.object),
            _ => false,
        }
    }

    fn emit_chain_base(self, expression: &Expression) -> String {
        if self.expression_can_be_chain_base(expression) {
            self.emit_expression(expression, PREC_LEFT_HAND_SIDE)
        } else {
            format!("({})", self.emit_expression(expression, PREC_LOWEST))
        }
    }

    fn expression_statement_requires_parentheses(self, expression: &Expression) -> bool {
        self.expression_statement_starts_with_forbidden_token(expression)
            || self.expression_statement_starts_with_let_bracket(expression)
    }

    fn expression_statement_starts_with_forbidden_token(self, expression: &Expression) -> bool {
        match expression {
            Expression::Object(_) | Expression::Function(_) | Expression::Class(_) => true,
            Expression::Sequence(sequence) => {
                sequence.expressions.first().is_some_and(|expression| {
                    self.expression_statement_starts_with_forbidden_token(expression)
                })
            }
            Expression::Conditional(expression) => {
                self.expression_statement_starts_with_forbidden_token(&expression.test)
            }
            Expression::Logical(expression) => {
                self.expression_statement_starts_with_forbidden_token(&expression.left)
            }
            Expression::Binary(expression) => {
                self.expression_statement_starts_with_forbidden_token(&expression.left)
            }
            Expression::Assignment(expression) => {
                matches!(expression.left, Expression::Object(_))
                    || self.expression_statement_starts_with_forbidden_token(&expression.left)
            }
            Expression::Update(expression) if !expression.prefix => {
                self.expression_statement_starts_with_forbidden_token(&expression.argument)
            }
            _ => false,
        }
    }

    fn expression_statement_starts_with_let_bracket(self, expression: &Expression) -> bool {
        match expression {
            Expression::Member(member) => {
                matches!(
                    (&member.object, &member.property),
                    (
                        Expression::Identifier(Identifier { name, .. }),
                        MemberProperty::Computed { .. }
                    ) if name == "let"
                ) || self.expression_statement_starts_with_let_bracket(&member.object)
            }
            Expression::Call(call) => {
                self.expression_statement_starts_with_let_bracket(&call.callee)
            }
            Expression::TaggedTemplate(tagged) => {
                self.expression_statement_starts_with_let_bracket(&tagged.tag)
            }
            _ => false,
        }
    }

    fn expression_can_be_chain_base(self, expression: &Expression) -> bool {
        match expression {
            Expression::Identifier(_)
            | Expression::PrivateIdentifier(_)
            | Expression::This(_)
            | Expression::Super(_)
            | Expression::Array(_)
            | Expression::Member(_)
            | Expression::Call(_)
            | Expression::MetaProperty(_) => true,
            Expression::New(new_expr) => new_expr.arguments.is_empty(),
            _ => false,
        }
    }

    fn emit_property_key(self, key: &PropertyKey) -> String {
        match key {
            PropertyKey::Identifier(identifier) => self.emit_identifier(identifier),
            PropertyKey::PrivateName(identifier) => format!("#{}", identifier.name),
            PropertyKey::String(string) => self.emit_string_literal_node(string),
            PropertyKey::Number(number) => number.raw.clone(),
            PropertyKey::Computed { expression, .. } => {
                format!(
                    "[{}]",
                    self.emit_expression(expression.as_ref(), PREC_LOWEST)
                )
            }
        }
    }

    fn emit_variable_initializer(self, expression: &Expression) -> String {
        match expression {
            Expression::Sequence(_) => {
                format!("({})", self.emit_expression(expression, PREC_LOWEST))
            }
            _ => self.emit_expression(expression, PREC_LOWEST),
        }
    }

    fn emit_field_initializer(self, expression: &Expression) -> String {
        match expression {
            Expression::Sequence(_) => {
                format!("({})", self.emit_expression(expression, PREC_LOWEST))
            }
            _ => self.emit_expression(expression, PREC_LOWEST),
        }
    }

    fn emit_logical_operand(
        self,
        expression: &Expression,
        enclosing_operator: LogicalOperator,
        min_prec: u8,
    ) -> String {
        if self.logical_mixing_requires_parentheses(expression, enclosing_operator) {
            format!("({})", self.emit_expression(expression, PREC_LOWEST))
        } else {
            self.emit_expression(expression, min_prec)
        }
    }

    fn logical_mixing_requires_parentheses(
        self,
        expression: &Expression,
        enclosing_operator: LogicalOperator,
    ) -> bool {
        let Expression::Logical(logical) = expression else {
            return false;
        };
        matches!(
            (enclosing_operator, logical.operator),
            (
                LogicalOperator::NullishCoalescing,
                LogicalOperator::And | LogicalOperator::Or
            ) | (
                LogicalOperator::And | LogicalOperator::Or,
                LogicalOperator::NullishCoalescing
            )
        )
    }

    fn emit_for_init(self, init: &ForInit) -> String {
        match init {
            ForInit::VariableDeclaration(declaration) => {
                self.emit_variable_declaration(declaration)
            }
            ForInit::Expression(expression) => self.emit_expression(expression, PREC_LOWEST),
        }
    }

    fn emit_for_left(self, left: &ForLeft) -> String {
        match left {
            ForLeft::VariableDeclaration(declaration) => {
                self.emit_variable_declaration(declaration)
            }
            ForLeft::Pattern(pattern) => self.emit_pattern(pattern),
            ForLeft::Expression(expression) => self.emit_for_left_expression(expression),
        }
    }

    fn emit_for_left_expression(self, expression: &Expression) -> String {
        if matches!(expression, Expression::Identifier(identifier) if identifier.name == "async") {
            format!("({})", self.emit_expression(expression, PREC_LOWEST))
        } else {
            self.emit_expression(expression, PREC_LOWEST)
        }
    }

    fn emit_identifier(self, identifier: &Identifier) -> String {
        identifier.name.clone()
    }

    fn emit_statement_in_statement_position(self, statement: &Statement, indent: usize) -> String {
        match statement {
            Statement::VariableDeclaration(declaration)
                if self.statement_position_requires_linebreak_let(declaration) =>
            {
                self.emit_linebreak_let_variable_declaration_statement(declaration, indent)
            }
            _ => self.emit_statement(statement, indent),
        }
    }

    fn emit_embedded_statement(self, statement: &Statement, indent: usize) -> String {
        match statement {
            Statement::Block(block) => format!(" {}", self.emit_block(block, indent)),
            _ => format!(
                "\n{}",
                self.emit_statement_in_statement_position(statement, indent + 1)
            ),
        }
    }

    fn statement_position_requires_linebreak_let(self, declaration: &VariableDeclaration) -> bool {
        declaration.kind == VariableKind::Let
            && declaration.declarations.first().is_some_and(|declarator| {
                matches!(
                    declarator.pattern,
                    Pattern::Identifier(_) | Pattern::Object(_)
                )
            })
    }

    fn emit_linebreak_let_variable_declaration_statement(
        self,
        declaration: &VariableDeclaration,
        indent: usize,
    ) -> String {
        let pad = self.indent(indent);
        let continuation_pad = self.indent(indent + 1);
        let declarations = declaration
            .declarations
            .iter()
            .map(|declarator| self.emit_variable_declarator(declarator))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{pad}let\n{continuation_pad}{declarations};")
    }

    fn emit_block(self, block: &BlockStatement, indent: usize) -> String {
        if block.body.is_empty() {
            return "{}".to_string();
        }

        let mut out = String::from("{\n");
        out.push_str(&self.emit_statement_list(&block.body, indent + 1));
        out.push('\n');
        out.push_str(&self.indent(indent));
        out.push('}');
        out
    }

    fn indent(self, indent: usize) -> String {
        "  ".repeat(indent)
    }

    fn emit_statement_list(self, statements: &[Statement], indent: usize) -> String {
        let mut out = String::new();
        let mut previous = None::<&Statement>;
        for statement in statements {
            if let Some(previous) = previous {
                out.push_str(self.statement_separator(previous, statement));
            }
            out.push_str(&self.emit_statement(statement, indent));
            previous = Some(statement);
        }
        out
    }

    fn statement_separator(self, previous: &Statement, next: &Statement) -> &'static str {
        if self.statement_pair_requires_compact_separator(previous, next) {
            ""
        } else {
            "\n"
        }
    }

    fn statement_pair_requires_compact_separator(
        self,
        previous: &Statement,
        next: &Statement,
    ) -> bool {
        self.statement_may_end_with_closing_brace(previous)
            && matches!(
                next,
                Statement::Expression(ExpressionStatement {
                    expression: Expression::Literal(Literal::RegExp(_)),
                    ..
                })
            )
    }

    fn statement_may_end_with_closing_brace(self, statement: &Statement) -> bool {
        match statement {
            Statement::Block(_)
            | Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_) => true,
            Statement::Labeled(statement) => {
                self.statement_may_end_with_closing_brace(&statement.body)
            }
            Statement::If(statement) => {
                self.statement_may_end_with_closing_brace(&statement.consequent)
                    || statement.alternate.as_ref().is_some_and(|alternate| {
                        self.statement_may_end_with_closing_brace(alternate)
                    })
            }
            Statement::While(statement) => {
                self.statement_may_end_with_closing_brace(&statement.body)
            }
            Statement::DoWhile(_) => false,
            Statement::For(statement) => match statement {
                ForStatement::Classic(statement) => {
                    self.statement_may_end_with_closing_brace(&statement.body)
                }
                ForStatement::In(statement) | ForStatement::Of(statement) => {
                    self.statement_may_end_with_closing_brace(&statement.body)
                }
            },
            Statement::Switch(_) | Statement::Try(_) => true,
            Statement::With(statement) => {
                self.statement_may_end_with_closing_brace(&statement.body)
            }
            _ => false,
        }
    }

    fn expression_precedence(self, expression: &Expression) -> u8 {
        match expression {
            Expression::Sequence(_) => PREC_SEQUENCE,
            Expression::Assignment(_) | Expression::ArrowFunction(_) => PREC_ASSIGNMENT,
            Expression::Yield(_) => PREC_YIELD,
            Expression::Conditional(_) => PREC_CONDITIONAL,
            Expression::Logical(expression) => match expression.operator {
                LogicalOperator::Or => PREC_LOGICAL_OR,
                LogicalOperator::NullishCoalescing => PREC_NULLISH,
                LogicalOperator::And => PREC_LOGICAL_AND,
            },
            Expression::Binary(expression) => match expression.operator {
                BinaryOperator::BitwiseOr => PREC_BIT_OR,
                BinaryOperator::BitwiseXor => PREC_BIT_XOR,
                BinaryOperator::BitwiseAnd => PREC_BIT_AND,
                BinaryOperator::Equality
                | BinaryOperator::StrictEquality
                | BinaryOperator::Inequality
                | BinaryOperator::StrictInequality => PREC_EQUALITY,
                BinaryOperator::LessThan
                | BinaryOperator::LessThanOrEqual
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterThanOrEqual
                | BinaryOperator::In
                | BinaryOperator::Instanceof => PREC_RELATIONAL,
                BinaryOperator::LeftShift
                | BinaryOperator::SignedRightShift
                | BinaryOperator::UnsignedRightShift => PREC_SHIFT,
                BinaryOperator::Add | BinaryOperator::Subtract => PREC_ADDITIVE,
                BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Modulo => {
                    PREC_MULTIPLICATIVE
                }
                BinaryOperator::Exponentiate => PREC_EXPONENTIATION,
            },
            Expression::Await(_) | Expression::Unary(_) => PREC_UNARY,
            Expression::Update(_) => PREC_UPDATE,
            Expression::New(_) => PREC_NEW,
            Expression::Call(_) | Expression::Member(_) | Expression::TaggedTemplate(_) => {
                PREC_LEFT_HAND_SIDE
            }
            Expression::Identifier(_)
            | Expression::PrivateIdentifier(_)
            | Expression::Literal(_)
            | Expression::This(_)
            | Expression::Super(_)
            | Expression::Array(_)
            | Expression::Object(_)
            | Expression::Function(_)
            | Expression::Class(_)
            | Expression::MetaProperty(_) => PREC_PRIMARY,
        }
    }

    fn emit_unary_operator(self, operator: UnaryOperator) -> &'static str {
        match operator {
            UnaryOperator::Delete => "delete ",
            UnaryOperator::Void => "void ",
            UnaryOperator::Typeof => "typeof ",
            UnaryOperator::Positive => "+",
            UnaryOperator::Negative => "-",
            UnaryOperator::LogicalNot => "!",
            UnaryOperator::BitNot => "~",
        }
    }

    fn emit_update_operator(self, operator: UpdateOperator) -> &'static str {
        match operator {
            UpdateOperator::Increment => "++",
            UpdateOperator::Decrement => "--",
        }
    }

    fn emit_binary_operator(self, operator: BinaryOperator) -> &'static str {
        match operator {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Modulo => "%",
            BinaryOperator::Exponentiate => "**",
            BinaryOperator::LeftShift => "<<",
            BinaryOperator::SignedRightShift => ">>",
            BinaryOperator::UnsignedRightShift => ">>>",
            BinaryOperator::LessThan => "<",
            BinaryOperator::LessThanOrEqual => "<=",
            BinaryOperator::GreaterThan => ">",
            BinaryOperator::GreaterThanOrEqual => ">=",
            BinaryOperator::Equality => "==",
            BinaryOperator::StrictEquality => "===",
            BinaryOperator::Inequality => "!=",
            BinaryOperator::StrictInequality => "!==",
            BinaryOperator::BitwiseAnd => "&",
            BinaryOperator::BitwiseOr => "|",
            BinaryOperator::BitwiseXor => "^",
            BinaryOperator::In => "in",
            BinaryOperator::Instanceof => "instanceof",
        }
    }

    fn emit_logical_operator(self, operator: LogicalOperator) -> &'static str {
        match operator {
            LogicalOperator::And => "&&",
            LogicalOperator::Or => "||",
            LogicalOperator::NullishCoalescing => "??",
        }
    }

    fn emit_assignment_operator(self, operator: AssignmentOperator) -> &'static str {
        match operator {
            AssignmentOperator::Assign => "=",
            AssignmentOperator::AddAssign => "+=",
            AssignmentOperator::SubAssign => "-=",
            AssignmentOperator::MulAssign => "*=",
            AssignmentOperator::DivAssign => "/=",
            AssignmentOperator::ModAssign => "%=",
            AssignmentOperator::PowAssign => "**=",
            AssignmentOperator::ShlAssign => "<<=",
            AssignmentOperator::SarAssign => ">>=",
            AssignmentOperator::ShrAssign => ">>>=",
            AssignmentOperator::AndAssign => "&=",
            AssignmentOperator::XorAssign => "^=",
            AssignmentOperator::OrAssign => "|=",
            AssignmentOperator::LogicalAndAssign => "&&=",
            AssignmentOperator::LogicalOrAssign => "||=",
            AssignmentOperator::NullishAssign => "??=",
        }
    }
}
