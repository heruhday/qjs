use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process,
};

use ::ast::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expectation {
    ParsePass,
    ParseFail,
}

#[derive(Debug)]
struct Metadata {
    expectation: Expectation,
    is_module: bool,
    is_strict: bool,
}

#[derive(Debug)]
struct FailureRecord {
    path: PathBuf,
    stage: &'static str,
    message: String,
}

#[derive(Debug, Default)]
struct Summary {
    total: usize,
    passed: usize,
    expected_parse_fail: usize,
    module_tests: usize,
    negative_cases_matched: usize,
    positive_cases_matched: usize,
    unexpected_parse_successes: Vec<PathBuf>,
    failures: Vec<FailureRecord>,
}

fn main() {
    let status = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .and_then(|handle| {
            handle
                .join()
                .map_err(|_| std::io::Error::other("worker thread panicked"))
        });

    match status {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("failed to run checker: {error}");
            process::exit(2);
        }
    }
}

fn run() -> i32 {
    let mut args = env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_test262_root);
    let failure_limit = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20);

    if !root.exists() {
        eprintln!("path does not exist: {}", root.display());
        return 2;
    }

    let files = match collect_js_files(&root) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("failed to collect test files: {error}");
            return 2;
        }
    };

    let mut summary = Summary::default();
    let mut failure_messages: BTreeMap<String, usize> = BTreeMap::new();

    for path in files {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("failed to read {}: {error}", path.display());
                return 2;
            }
        };

        let metadata = extract_metadata(&source);
        summary.total += 1;
        if metadata.is_module {
            summary.module_tests += 1;
        }
        if metadata.expectation == Expectation::ParseFail {
            summary.expected_parse_fail += 1;
        }

        if metadata.expectation == Expectation::ParseFail {
            match parse_with_options(
                &source,
                ParseOptions {
                    is_module: metadata.is_module,
                    is_strict: metadata.is_strict,
                },
            ) {
                Ok(_) => {
                    summary.unexpected_parse_successes.push(path);
                }
                Err(_) => {
                    summary.negative_cases_matched += 1;
                    summary.passed += 1;
                }
            }
            continue;
        }

        // 1. js -> ast (1st)
        let first_ast = match parse_with_options(
            &source,
            ParseOptions {
                is_module: metadata.is_module,
                is_strict: metadata.is_strict,
            },
        ) {
            Ok(program) => program,
            Err(error) => {
                *failure_messages.entry(error.message.clone()).or_default() += 1;
                summary.failures.push(FailureRecord {
                    path: path.clone(),
                    stage: "parse",
                    message: error.to_string(),
                });
                continue;
            }
        };

        // 2. ast -> js
        let emitted = program_to_js(&first_ast);

        // 3. js -> ast (2nd)
        let second_ast = match parse_with_options(
            &emitted,
            ParseOptions {
                is_module: metadata.is_module,
                is_strict: metadata.is_strict,
            },
        ) {
            Ok(program) => program,
            Err(error) => {
                *failure_messages.entry(error.message.clone()).or_default() += 1;
                summary.failures.push(FailureRecord {
                    path: path.clone(),
                    stage: "reparse_emitted",
                    message: format!("{error}\nemitted:\n{emitted}"),
                });
                continue;
            }
        };

        // 4. check if the two asts (1st == 2nd) are equal
        if !structurally_equal_programs(&first_ast, &second_ast) {
            let reason = "ASTs are not equal after round-trip".to_string();
            *failure_messages.entry(reason.clone()).or_default() += 1;
            summary.failures.push(FailureRecord {
                path,
                stage: "ast_equality",
                message: format!(
                    "{}\nOriginal source:\n{}\nEmitted JS:\n{}",
                    reason, source, emitted
                ),
            });
            continue;
        }

        summary.positive_cases_matched += 1;
        summary.passed += 1;
    }

    let failed = summary.total - summary.passed;
    let percent = if summary.total == 0 {
        100.0
    } else {
        (summary.passed as f64 * 100.0) / summary.total as f64
    };
    let parse_pass_total = summary.total - summary.expected_parse_fail;

    println!("root: {}", root.display());
    println!("total: {}", summary.total);
    println!("passed: {}", summary.passed);
    println!("failed: {}", failed);
    println!("pass rate: {:.2}%", percent);
    println!("parse-negative tests: {}", summary.expected_parse_fail);
    println!("module-flag tests: {}", summary.module_tests);
    println!(
        "positive round-trip cases matched: {}/{}",
        summary.positive_cases_matched, parse_pass_total
    );
    println!(
        "negative parse cases matched: {}/{}",
        summary.negative_cases_matched, summary.expected_parse_fail
    );
    println!(
        "unexpected parse successes: {}",
        summary.unexpected_parse_successes.len()
    );
    println!("unexpected failures: {}", summary.failures.len());

    if !summary.failures.is_empty() {
        println!();
        println!("top unexpected failure messages:");
        let mut top_messages: Vec<_> = failure_messages.into_iter().collect();
        top_messages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (message, count) in top_messages.into_iter().take(failure_limit) {
            println!("  {count:>5}  {message}");
        }

        println!();
        println!("sample unexpected failures:");
        for failure in summary.failures.iter().take(failure_limit) {
            println!(
                "  {} [{}] :: {}",
                failure.path.display(),
                failure.stage,
                failure.message
            );
        }
    }

    if !summary.unexpected_parse_successes.is_empty() {
        println!();
        println!("sample unexpected parse successes:");
        for path in summary
            .unexpected_parse_successes
            .iter()
            .take(failure_limit)
        {
            println!("  {}", path.display());
        }
    }

    if failed > 0 {
        return 1;
    }

    0
}

fn default_test262_root() -> PathBuf {
    let workspace_test262 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test262/test");
    if Path::new("test262/test").exists() {
        PathBuf::from("test262/test")
    } else {
        workspace_test262
    }
}

fn collect_js_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                stack.push(entry?.path());
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("js")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("_FIXTURE"))
        {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn extract_metadata(source: &str) -> Metadata {
    let frontmatter = source
        .find("/*---")
        .and_then(|start| source[start + 5..].split_once("---*/"))
        .map(|(frontmatter, _)| frontmatter)
        .unwrap_or("");

    let mut is_module = false;
    let mut is_strict = false;
    let mut in_negative = false;
    let mut in_flags = false;
    let mut negative_phase = None::<String>;

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        if in_flags {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_flags = false;
            } else if let Some(flag) = trimmed.strip_prefix('-') {
                match flag.trim() {
                    "module" => is_module = true,
                    "onlyStrict" => is_strict = true,
                    _ => {}
                }
                continue;
            }
        }

        if trimmed == "negative:" {
            in_negative = true;
            continue;
        }

        if in_negative {
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_negative = false;
            } else if let Some(value) = trimmed.strip_prefix("phase:") {
                negative_phase = Some(value.trim().to_string());
                continue;
            }
        }

        if trimmed == "flags:" {
            in_flags = true;
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("flags: [") {
            in_flags = false;
            for flag in value.trim_end_matches(']').split(',') {
                match flag.trim() {
                    "module" => is_module = true,
                    "onlyStrict" => is_strict = true,
                    _ => {}
                }
            }
            continue;
        }
    }

    let expectation = match negative_phase.as_deref() {
        Some("parse") | Some("early") => Expectation::ParseFail,
        _ => Expectation::ParsePass,
    };

    Metadata {
        expectation,
        is_module,
        is_strict,
    }
}

fn structurally_equal_programs(left: &Program, right: &Program) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    normalize_program(&mut left);
    normalize_program(&mut right);
    left == right
}

fn clear_span(span: &mut Span) {
    *span = Span::default();
}

fn normalize_program(program: &mut Program) {
    for statement in &mut program.body {
        normalize_statement(statement);
    }
    clear_span(&mut program.span);
}

fn normalize_statement(statement: &mut Statement) {
    match statement {
        Statement::Directive(directive) => clear_span(&mut directive.span),
        Statement::Empty(span) | Statement::Debugger(span) => clear_span(span),
        Statement::Block(block) => normalize_block_statement(block),
        Statement::Labeled(statement) => {
            normalize_identifier(&mut statement.label);
            normalize_statement(statement.body.as_mut());
            clear_span(&mut statement.span);
        }
        Statement::ImportDeclaration(declaration) => normalize_import_declaration(declaration),
        Statement::ExportDeclaration(declaration) => normalize_export_declaration(declaration),
        Statement::VariableDeclaration(declaration) => {
            normalize_variable_declaration(declaration);
        }
        Statement::FunctionDeclaration(function) => normalize_function(function),
        Statement::ClassDeclaration(class) => normalize_class(class),
        Statement::If(statement) => normalize_if_statement(statement),
        Statement::While(statement) => normalize_while_statement(statement),
        Statement::DoWhile(statement) => normalize_do_while_statement(statement),
        Statement::For(statement) => normalize_for_statement(statement),
        Statement::Switch(statement) => normalize_switch_statement(statement),
        Statement::Return(statement) => normalize_return_statement(statement),
        Statement::Break(statement) | Statement::Continue(statement) => {
            normalize_jump_statement(statement);
        }
        Statement::Throw(statement) => normalize_throw_statement(statement),
        Statement::Try(statement) => normalize_try_statement(statement),
        Statement::With(statement) => normalize_with_statement(statement),
        Statement::Expression(statement) => normalize_expression_statement(statement),
    }
}

fn normalize_block_statement(block: &mut BlockStatement) {
    for statement in &mut block.body {
        normalize_statement(statement);
    }
    clear_span(&mut block.span);
}

fn normalize_expression_statement(statement: &mut ExpressionStatement) {
    normalize_expression(&mut statement.expression);
    clear_span(&mut statement.span);
}

fn normalize_import_declaration(declaration: &mut ImportDeclaration) {
    if let Some(clause) = &mut declaration.clause {
        normalize_import_clause(clause);
    }
    normalize_string_literal(&mut declaration.source);
    if let Some(attributes) = &mut declaration.attributes {
        normalize_expression(attributes);
    }
    clear_span(&mut declaration.span);
}

fn normalize_import_clause(clause: &mut ImportClause) {
    match clause {
        ImportClause::Default(identifier) => normalize_identifier(identifier),
        ImportClause::Namespace { default, namespace } => {
            if let Some(default) = default {
                normalize_identifier(default);
            }
            normalize_identifier(namespace);
        }
        ImportClause::Named {
            default,
            specifiers,
        } => {
            if let Some(default) = default {
                normalize_identifier(default);
            }
            for specifier in specifiers {
                normalize_import_specifier(specifier);
            }
        }
    }
}

fn normalize_import_specifier(specifier: &mut ImportSpecifier) {
    normalize_module_export_name(&mut specifier.imported);
    normalize_identifier(&mut specifier.local);
    clear_span(&mut specifier.span);
}

fn normalize_export_declaration(declaration: &mut ExportDeclaration) {
    match declaration {
        ExportDeclaration::All(declaration) => normalize_export_all_declaration(declaration),
        ExportDeclaration::Named(declaration) => normalize_export_named_declaration(declaration),
        ExportDeclaration::Default(declaration) => {
            normalize_export_default_declaration(declaration);
        }
        ExportDeclaration::Declaration(declaration) => {
            normalize_exported_declaration(declaration);
        }
    }
}

fn normalize_export_all_declaration(declaration: &mut ExportAllDeclaration) {
    if let Some(exported) = &mut declaration.exported {
        normalize_module_export_name(exported);
    }
    normalize_string_literal(&mut declaration.source);
    if let Some(attributes) = &mut declaration.attributes {
        normalize_expression(attributes);
    }
    clear_span(&mut declaration.span);
}

fn normalize_export_named_declaration(declaration: &mut ExportNamedDeclaration) {
    for specifier in &mut declaration.specifiers {
        normalize_export_specifier(specifier);
    }
    if let Some(source) = &mut declaration.source {
        normalize_string_literal(source);
    }
    if let Some(attributes) = &mut declaration.attributes {
        normalize_expression(attributes);
    }
    clear_span(&mut declaration.span);
}

fn normalize_export_default_declaration(declaration: &mut ExportDefaultDeclaration) {
    normalize_export_default_kind(&mut declaration.declaration);
    clear_span(&mut declaration.span);
}

fn normalize_export_default_kind(kind: &mut ExportDefaultKind) {
    match kind {
        ExportDefaultKind::Function(function) => normalize_function(function),
        ExportDefaultKind::Class(class) => normalize_class(class),
        ExportDefaultKind::Expression(expression) => normalize_expression(expression),
    }
}

fn normalize_exported_declaration(declaration: &mut ExportedDeclaration) {
    match declaration {
        ExportedDeclaration::Variable(declaration) => normalize_variable_declaration(declaration),
        ExportedDeclaration::Function(function) => normalize_function(function),
        ExportedDeclaration::Class(class) => normalize_class(class),
    }
}

fn normalize_export_specifier(specifier: &mut ExportSpecifier) {
    normalize_module_export_name(&mut specifier.local);
    normalize_module_export_name(&mut specifier.exported);
    clear_span(&mut specifier.span);
}

fn normalize_module_export_name(name: &mut ModuleExportName) {
    match name {
        ModuleExportName::Identifier(identifier) => normalize_identifier(identifier),
        ModuleExportName::String(string) => normalize_string_literal(string),
    }
}

fn normalize_variable_declaration(declaration: &mut VariableDeclaration) {
    for declarator in &mut declaration.declarations {
        normalize_variable_declarator(declarator);
    }
    clear_span(&mut declaration.span);
}

fn normalize_variable_declarator(declarator: &mut VariableDeclarator) {
    normalize_pattern(&mut declarator.pattern);
    if let Some(init) = &mut declarator.init {
        normalize_expression(init);
    }
    clear_span(&mut declarator.span);
}

fn normalize_function(function: &mut Function) {
    if let Some(id) = &mut function.id {
        normalize_identifier(id);
    }
    for param in &mut function.params {
        normalize_pattern(param);
    }
    normalize_block_statement(&mut function.body);
    clear_span(&mut function.span);
}

fn normalize_class(class: &mut Class) {
    for decorator in &mut class.decorators {
        normalize_expression(decorator);
    }
    if let Some(id) = &mut class.id {
        normalize_identifier(id);
    }
    if let Some(super_class) = &mut class.super_class {
        normalize_expression(super_class);
    }
    for element in &mut class.body {
        normalize_class_element(element);
    }
    clear_span(&mut class.span);
}

fn normalize_class_element(element: &mut ClassElement) {
    match element {
        ClassElement::Empty(span) => clear_span(span),
        ClassElement::StaticBlock(block) => normalize_block_statement(block),
        ClassElement::Method(method) => normalize_class_method(method),
        ClassElement::Field(field) => normalize_class_field(field),
    }
}

fn normalize_class_method(method: &mut ClassMethod) {
    for decorator in &mut method.decorators {
        normalize_expression(decorator);
    }
    normalize_property_key(&mut method.key);
    normalize_function(&mut method.value);
    clear_span(&mut method.span);
}

fn normalize_class_field(field: &mut ClassField) {
    for decorator in &mut field.decorators {
        normalize_expression(decorator);
    }
    normalize_property_key(&mut field.key);
    if let Some(value) = &mut field.value {
        normalize_expression(value);
    }
    clear_span(&mut field.span);
}

fn normalize_if_statement(statement: &mut IfStatement) {
    normalize_expression(&mut statement.test);
    normalize_statement(statement.consequent.as_mut());
    if let Some(alternate) = &mut statement.alternate {
        normalize_statement(alternate.as_mut());
    }
    clear_span(&mut statement.span);
}

fn normalize_while_statement(statement: &mut WhileStatement) {
    normalize_expression(&mut statement.test);
    normalize_statement(statement.body.as_mut());
    clear_span(&mut statement.span);
}

fn normalize_do_while_statement(statement: &mut DoWhileStatement) {
    normalize_statement(statement.body.as_mut());
    normalize_expression(&mut statement.test);
    clear_span(&mut statement.span);
}

fn normalize_for_statement(statement: &mut ForStatement) {
    match statement {
        ForStatement::Classic(statement) => normalize_for_classic_statement(statement),
        ForStatement::In(statement) | ForStatement::Of(statement) => {
            normalize_for_each_statement(statement);
        }
    }
}

fn normalize_for_classic_statement(statement: &mut ForClassicStatement) {
    if let Some(init) = &mut statement.init {
        normalize_for_init(init);
    }
    if let Some(test) = &mut statement.test {
        normalize_expression(test);
    }
    if let Some(update) = &mut statement.update {
        normalize_expression(update);
    }
    normalize_statement(statement.body.as_mut());
    clear_span(&mut statement.span);
}

fn normalize_for_init(init: &mut ForInit) {
    match init {
        ForInit::VariableDeclaration(declaration) => normalize_variable_declaration(declaration),
        ForInit::Expression(expression) => normalize_expression(expression),
    }
}

fn normalize_for_each_statement(statement: &mut ForEachStatement) {
    normalize_for_left(&mut statement.left);
    normalize_expression(&mut statement.right);
    normalize_statement(statement.body.as_mut());
    clear_span(&mut statement.span);
}

fn normalize_for_left(left: &mut ForLeft) {
    match left {
        ForLeft::VariableDeclaration(declaration) => normalize_variable_declaration(declaration),
        ForLeft::Pattern(pattern) => normalize_pattern(pattern),
        ForLeft::Expression(expression) => normalize_expression(expression),
    }
}

fn normalize_switch_statement(statement: &mut SwitchStatement) {
    normalize_expression(&mut statement.discriminant);
    for case in &mut statement.cases {
        normalize_switch_case(case);
    }
    clear_span(&mut statement.span);
}

fn normalize_switch_case(case: &mut SwitchCase) {
    if let Some(test) = &mut case.test {
        normalize_expression(test);
    }
    for statement in &mut case.consequent {
        normalize_statement(statement);
    }
    clear_span(&mut case.span);
}

fn normalize_return_statement(statement: &mut ReturnStatement) {
    if let Some(argument) = &mut statement.argument {
        normalize_expression(argument);
    }
    clear_span(&mut statement.span);
}

fn normalize_jump_statement(statement: &mut JumpStatement) {
    if let Some(label) = &mut statement.label {
        normalize_identifier(label);
    }
    clear_span(&mut statement.span);
}

fn normalize_throw_statement(statement: &mut ThrowStatement) {
    normalize_expression(&mut statement.argument);
    clear_span(&mut statement.span);
}

fn normalize_try_statement(statement: &mut TryStatement) {
    normalize_block_statement(&mut statement.block);
    if let Some(handler) = &mut statement.handler {
        normalize_catch_clause(handler);
    }
    if let Some(finalizer) = &mut statement.finalizer {
        normalize_block_statement(finalizer);
    }
    clear_span(&mut statement.span);
}

fn normalize_catch_clause(clause: &mut CatchClause) {
    if let Some(param) = &mut clause.param {
        normalize_pattern(param);
    }
    normalize_block_statement(&mut clause.body);
    clear_span(&mut clause.span);
}

fn normalize_with_statement(statement: &mut WithStatement) {
    normalize_expression(&mut statement.object);
    normalize_statement(statement.body.as_mut());
    clear_span(&mut statement.span);
}

fn normalize_identifier(identifier: &mut Identifier) {
    clear_span(&mut identifier.span);
}

fn normalize_pattern(pattern: &mut Pattern) {
    match pattern {
        Pattern::Identifier(identifier) => normalize_identifier(identifier),
        Pattern::Array(pattern) => normalize_array_pattern(pattern),
        Pattern::Object(pattern) => normalize_object_pattern(pattern),
        Pattern::Rest(pattern) => normalize_rest_pattern(pattern),
        Pattern::Assignment(pattern) => normalize_assignment_pattern(pattern),
    }
}

fn normalize_array_pattern(pattern: &mut ArrayPattern) {
    for element in pattern.elements.iter_mut().flatten() {
        normalize_pattern(element);
    }
    clear_span(&mut pattern.span);
}

fn normalize_object_pattern(pattern: &mut ObjectPattern) {
    for property in &mut pattern.properties {
        normalize_object_pattern_property(property);
    }
    clear_span(&mut pattern.span);
}

fn normalize_object_pattern_property(property: &mut ObjectPatternProperty) {
    match property {
        ObjectPatternProperty::Property {
            key, value, span, ..
        } => {
            normalize_property_key(key);
            normalize_pattern(value.as_mut());
            clear_span(span);
        }
        ObjectPatternProperty::Rest { argument, span } => {
            normalize_pattern(argument.as_mut());
            clear_span(span);
        }
    }
}

fn normalize_rest_pattern(pattern: &mut RestPattern) {
    normalize_pattern(pattern.argument.as_mut());
    clear_span(&mut pattern.span);
}

fn normalize_assignment_pattern(pattern: &mut AssignmentPattern) {
    normalize_pattern(pattern.left.as_mut());
    normalize_expression(&mut pattern.right);
    clear_span(&mut pattern.span);
}

fn normalize_expression(expression: &mut Expression) {
    match expression {
        Expression::Identifier(identifier) | Expression::PrivateIdentifier(identifier) => {
            normalize_identifier(identifier);
        }
        Expression::Literal(literal) => normalize_literal(literal),
        Expression::This(span) | Expression::Super(span) => clear_span(span),
        Expression::Array(array) => normalize_array_expression(array),
        Expression::Object(object) => normalize_object_expression(object),
        Expression::Function(function) => normalize_function(function.as_mut()),
        Expression::ArrowFunction(function) => normalize_arrow_function(function.as_mut()),
        Expression::Class(class) => normalize_class(class.as_mut()),
        Expression::TaggedTemplate(expression) => {
            normalize_tagged_template_expression(expression.as_mut());
        }
        Expression::MetaProperty(expression) => {
            normalize_meta_property_expression(expression.as_mut());
        }
        Expression::Yield(expression) => normalize_yield_expression(expression.as_mut()),
        Expression::Await(expression) => normalize_await_expression(expression.as_mut()),
        Expression::Unary(expression) => normalize_unary_expression(expression.as_mut()),
        Expression::Update(expression) => normalize_update_expression(expression.as_mut()),
        Expression::Binary(expression) => normalize_binary_expression(expression.as_mut()),
        Expression::Logical(expression) => normalize_logical_expression(expression.as_mut()),
        Expression::Assignment(expression) => {
            normalize_assignment_expression(expression.as_mut());
        }
        Expression::Conditional(expression) => {
            normalize_conditional_expression(expression.as_mut());
        }
        Expression::Sequence(expression) => normalize_sequence_expression(expression),
        Expression::Call(expression) => normalize_call_expression(expression.as_mut()),
        Expression::Member(expression) => normalize_member_expression(expression.as_mut()),
        Expression::New(expression) => normalize_new_expression(expression.as_mut()),
    }
}

fn normalize_arrow_function(function: &mut ArrowFunction) {
    for param in &mut function.params {
        normalize_pattern(param);
    }
    normalize_arrow_body(&mut function.body);
    clear_span(&mut function.span);
}

fn normalize_arrow_body(body: &mut ArrowBody) {
    match body {
        ArrowBody::Expression(expression) => normalize_expression(expression.as_mut()),
        ArrowBody::Block(block) => normalize_block_statement(block),
    }
}

fn normalize_tagged_template_expression(expression: &mut TaggedTemplateExpression) {
    normalize_expression(&mut expression.tag);
    normalize_template_literal(&mut expression.quasi);
    clear_span(&mut expression.span);
}

fn normalize_meta_property_expression(expression: &mut MetaPropertyExpression) {
    normalize_identifier(&mut expression.meta);
    normalize_identifier(&mut expression.property);
    clear_span(&mut expression.span);
}

fn normalize_yield_expression(expression: &mut YieldExpression) {
    if let Some(argument) = &mut expression.argument {
        normalize_expression(argument);
    }
    clear_span(&mut expression.span);
}

fn normalize_await_expression(expression: &mut AwaitExpression) {
    normalize_expression(&mut expression.argument);
    clear_span(&mut expression.span);
}

fn normalize_literal(literal: &mut Literal) {
    match literal {
        Literal::Null(span) => clear_span(span),
        Literal::Boolean(literal) => normalize_boolean_literal(literal),
        Literal::Number(literal) => normalize_number_literal(literal),
        Literal::String(literal) => normalize_string_literal(literal),
        Literal::Template(literal) => normalize_template_literal(literal),
        Literal::RegExp(literal) => normalize_regexp_literal(literal),
    }
}

fn normalize_boolean_literal(literal: &mut BooleanLiteral) {
    clear_span(&mut literal.span);
}

fn normalize_number_literal(literal: &mut NumberLiteral) {
    clear_span(&mut literal.span);
}

fn normalize_string_literal(literal: &mut StringLiteral) {
    clear_span(&mut literal.span);
}

fn normalize_template_literal(literal: &mut TemplateLiteral) {
    clear_span(&mut literal.span);
}

fn normalize_regexp_literal(literal: &mut RegExpLiteral) {
    clear_span(&mut literal.span);
}

fn normalize_array_expression(array: &mut ArrayExpression) {
    for element in array.elements.iter_mut().flatten() {
        normalize_array_element(element);
    }
    clear_span(&mut array.span);
}

fn normalize_array_element(element: &mut ArrayElement) {
    match element {
        ArrayElement::Expression(expression) => normalize_expression(expression),
        ArrayElement::Spread { argument, span } => {
            normalize_expression(argument);
            clear_span(span);
        }
    }
}

fn normalize_object_expression(object: &mut ObjectExpression) {
    for property in &mut object.properties {
        normalize_object_property(property);
    }
    clear_span(&mut object.span);
}

fn normalize_object_property(property: &mut ObjectProperty) {
    match property {
        ObjectProperty::Property {
            key, value, span, ..
        } => {
            normalize_property_key(key);
            normalize_expression(value);
            clear_span(span);
        }
        ObjectProperty::Spread { argument, span } => {
            normalize_expression(argument);
            clear_span(span);
        }
    }
}

fn normalize_property_key(key: &mut PropertyKey) {
    match key {
        PropertyKey::Identifier(identifier) | PropertyKey::PrivateName(identifier) => {
            normalize_identifier(identifier);
        }
        PropertyKey::String(string) => normalize_string_literal(string),
        PropertyKey::Number(number) => normalize_number_literal(number),
        PropertyKey::Computed { expression, span } => {
            normalize_expression(expression.as_mut());
            clear_span(span);
        }
    }
}

fn normalize_unary_expression(expression: &mut UnaryExpression) {
    normalize_expression(&mut expression.argument);
    clear_span(&mut expression.span);
}

fn normalize_update_expression(expression: &mut UpdateExpression) {
    normalize_expression(&mut expression.argument);
    clear_span(&mut expression.span);
}

fn normalize_binary_expression(expression: &mut BinaryExpression) {
    normalize_expression(&mut expression.left);
    normalize_expression(&mut expression.right);
    clear_span(&mut expression.span);
}

fn normalize_logical_expression(expression: &mut LogicalExpression) {
    normalize_expression(&mut expression.left);
    normalize_expression(&mut expression.right);
    clear_span(&mut expression.span);
}

fn normalize_assignment_expression(expression: &mut AssignmentExpression) {
    normalize_expression(&mut expression.left);
    normalize_expression(&mut expression.right);
    clear_span(&mut expression.span);
}

fn normalize_conditional_expression(expression: &mut ConditionalExpression) {
    normalize_expression(&mut expression.test);
    normalize_expression(&mut expression.consequent);
    normalize_expression(&mut expression.alternate);
    clear_span(&mut expression.span);
}

fn normalize_sequence_expression(expression: &mut SequenceExpression) {
    for part in &mut expression.expressions {
        normalize_expression(part);
    }
    clear_span(&mut expression.span);
}

fn normalize_call_expression(expression: &mut CallExpression) {
    normalize_expression(&mut expression.callee);
    for argument in &mut expression.arguments {
        normalize_call_argument(argument);
    }
    clear_span(&mut expression.span);
}

fn normalize_call_argument(argument: &mut CallArgument) {
    match argument {
        CallArgument::Expression(expression) => normalize_expression(expression),
        CallArgument::Spread { argument, span } => {
            normalize_expression(argument);
            clear_span(span);
        }
    }
}

fn normalize_member_expression(expression: &mut MemberExpression) {
    normalize_expression(&mut expression.object);
    normalize_member_property(&mut expression.property);
    clear_span(&mut expression.span);
}

fn normalize_member_property(property: &mut MemberProperty) {
    match property {
        MemberProperty::Identifier(identifier) | MemberProperty::PrivateName(identifier) => {
            normalize_identifier(identifier);
        }
        MemberProperty::Computed { expression, span } => {
            normalize_expression(expression.as_mut());
            clear_span(span);
        }
    }
}

fn normalize_new_expression(expression: &mut NewExpression) {
    normalize_expression(&mut expression.callee);
    for argument in &mut expression.arguments {
        normalize_call_argument(argument);
    }
    clear_span(&mut expression.span);
}
