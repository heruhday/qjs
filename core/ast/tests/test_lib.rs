#[cfg(test)]
mod tests {
    use ::ast::*;

    fn parse_single_expression(source: &str) -> Expression {
        let program = parse(source).unwrap();
        let [Statement::Expression(statement)] = program.body.as_slice() else {
            panic!("expected a single expression statement");
        };
        statement.expression.clone()
    }

    #[test]
    fn lexer_handles_keywords_comments_and_line_breaks() {
        let tokens = lex("let total = 1 + 2;\n// note\nreturn total;").unwrap();
        let tags: Vec<_> = tokens.iter().map(Token::tag).collect();
        assert_eq!(
            tags,
            vec![
                TokenTag::Let,
                TokenTag::Identifier,
                TokenTag::Assign,
                TokenTag::Number,
                TokenTag::Add,
                TokenTag::Number,
                TokenTag::Semicolon,
                TokenTag::Return,
                TokenTag::Identifier,
                TokenTag::Semicolon,
                TokenTag::Eof,
            ]
        );
        assert!(tokens[7].leading_line_break);
    }

    #[test]
    fn parser_handles_function_with_destructuring_patterns() {
        let program =
            parse("function sum({ left, right = 2 }, [first, ...rest]) { return left + first; }")
                .unwrap();
        println!("Program: {:#?}", program);
        let [Statement::FunctionDeclaration(function)] = program.body.as_slice() else {
            panic!("expected a single function declaration");
        };

        assert_eq!(function.id.as_ref().map(|id| id.name.as_str()), Some("sum"));
        assert_eq!(function.params.len(), 2);
        assert!(matches!(function.params[0], Pattern::Object(_)));
        assert!(matches!(function.params[1], Pattern::Array(_)));
        assert_eq!(function.body.body.len(), 1);
        assert!(matches!(function.body.body[0], Statement::Return(_)));
    }

    #[test]
    fn parser_handles_for_of_and_spread_expressions() {
        let program =
            parse("for (let item of items) { values = [...items, item]; call(...values); }")
                .unwrap();

        let [Statement::For(ForStatement::Of(statement))] = program.body.as_slice() else {
            panic!("expected a single for-of statement");
        };

        assert!(matches!(
            statement.left,
            ForLeft::VariableDeclaration(VariableDeclaration {
                kind: VariableKind::Let,
                ..
            })
        ));
        assert_eq!(statement.body.span().start.line, 1);

        let Statement::Block(block) = statement.body.as_ref() else {
            panic!("expected block body");
        };
        assert_eq!(block.body.len(), 2);
    }

    #[test]
    fn parser_handles_arrow_functions_regex_and_class_syntax() {
        let program = parse(
            "const fnRef = (a = /x+/g, { b }) => ({ valueOf() { return b; }, answer: a }); class Box { value = 1; get size() { return this.value; } }",
        )
        .unwrap();

        assert_eq!(program.body.len(), 2);
        assert!(matches!(
            program.body[0],
            Statement::VariableDeclaration(VariableDeclaration {
                declarations: _,
                ..
            })
        ));
        assert!(matches!(program.body[1], Statement::ClassDeclaration(_)));
    }

    #[test]
    fn parser_handles_regexp_legacy_accessor_sample() {
        let source = include_str!(
            "../../../test262/test/annexB/built-ins/RegExp/legacy-accessors/index/prop-desc.js"
        );
        parse(source).unwrap();
    }

    #[test]
    fn parser_handles_try_catch_finally_and_interpolated_templates() {
        let program = parse(
            "try { work(); } catch ({ message }) { log(`error: ${message}`); } finally { cleanup(); }",
        )
        .unwrap();

        let [Statement::Try(statement)] = program.body.as_slice() else {
            panic!("expected try statement");
        };

        assert!(statement.handler.is_some());
        assert!(statement.finalizer.is_some());
    }

    #[test]
    fn parser_handles_tagged_template_sample() {
        let source = include_str!(
            "../../../test262/test/language/expressions/template-literal/tv-character-escape-sequence.js"
        );
        parse(source).unwrap();
    }

    #[test]
    fn parser_handles_labels_switch_and_do_while() {
        parse("outer: do { switch (value) { case 1: break outer; default: continue outer; } } while (ready);")
            .unwrap();
    }

    #[test]
    fn parser_handles_new_target_and_numeric_literal_members() {
        parse("function C() { return new.target; } 0..toString(10);").unwrap();
        parse_with_options(
            "import.meta; import('x');",
            ParseOptions {
                is_module: true,
                is_strict: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn parser_handles_destructuring_assignment_patterns() {
        parse("({ x = 1, nested: [value, ...rest] } = source);").unwrap();
        parse("[{ get y() {}, set y(value) {} }.y, ...{}[key]] = values;").unwrap();
    }

    #[test]
    fn parser_preserves_expression_precedence_and_associativity() {
        let program = parse("a + b * c ** d; a ** b ** c;").unwrap();

        let Statement::Expression(first) = &program.body[0] else {
            panic!("expected first statement to be an expression");
        };
        let Expression::Binary(add) = &first.expression else {
            panic!("expected additive expression");
        };
        assert_eq!(add.operator, BinaryOperator::Add);
        let Expression::Binary(multiply) = &add.right else {
            panic!("expected multiplicative rhs");
        };
        assert_eq!(multiply.operator, BinaryOperator::Multiply);
        let Expression::Binary(exponentiate) = &multiply.right else {
            panic!("expected exponentiation rhs");
        };
        assert_eq!(exponentiate.operator, BinaryOperator::Exponentiate);

        let Statement::Expression(second) = &program.body[1] else {
            panic!("expected second statement to be an expression");
        };
        let Expression::Binary(exponentiate) = &second.expression else {
            panic!("expected exponentiation expression");
        };
        assert_eq!(exponentiate.operator, BinaryOperator::Exponentiate);
        let Expression::Binary(rhs) = &exponentiate.right else {
            panic!("expected exponentiation to associate to the right");
        };
        assert_eq!(rhs.operator, BinaryOperator::Exponentiate);
    }

    #[test]
    fn parser_parses_conditional_expressions_with_lower_precedence_than_binary_ops() {
        let program = parse("a + b ? c : d ? e : f;").unwrap();

        let [Statement::Expression(statement)] = program.body.as_slice() else {
            panic!("expected a single expression statement");
        };
        let Expression::Conditional(outer) = &statement.expression else {
            panic!("expected outer conditional expression");
        };
        assert!(matches!(
            outer.test,
            Expression::Binary(ref expression) if expression.operator == BinaryOperator::Add
        ));
        let Expression::Conditional(inner) = &outer.alternate else {
            panic!("expected nested conditional in alternate branch");
        };
        assert!(matches!(
            inner.test,
            Expression::Identifier(ref identifier) if identifier.name == "d"
        ));
    }

    #[test]
    fn parser_pratt_preserves_relational_and_equality_precedence() {
        let expression = parse_single_expression("1 + 2 > 3;");
        let Expression::Binary(greater_than) = expression else {
            panic!("expected relational expression");
        };
        assert_eq!(greater_than.operator, BinaryOperator::GreaterThan);
        assert!(matches!(
            greater_than.left,
            Expression::Binary(ref add) if add.operator == BinaryOperator::Add
        ));

        let expression = parse_single_expression("1 > 2 + 3;");
        let Expression::Binary(greater_than) = expression else {
            panic!("expected relational expression");
        };
        assert_eq!(greater_than.operator, BinaryOperator::GreaterThan);
        assert!(matches!(
            greater_than.right,
            Expression::Binary(ref add) if add.operator == BinaryOperator::Add
        ));

        let expression = parse_single_expression("1 + 2 == 3;");
        let Expression::Binary(equality) = expression else {
            panic!("expected equality expression");
        };
        assert_eq!(equality.operator, BinaryOperator::Equality);
        assert!(matches!(
            equality.left,
            Expression::Binary(ref add) if add.operator == BinaryOperator::Add
        ));

        let expression = parse_single_expression("1 == 2 + 3;");
        let Expression::Binary(equality) = expression else {
            panic!("expected equality expression");
        };
        assert_eq!(equality.operator, BinaryOperator::Equality);
        assert!(matches!(
            equality.right,
            Expression::Binary(ref add) if add.operator == BinaryOperator::Add
        ));
    }

    #[test]
    fn parser_pratt_preserves_logical_and_bitwise_layers() {
        let expression = parse_single_expression("a || b && c | d ^ e & f;");

        let Expression::Logical(or_expr) = expression else {
            panic!("expected logical-or expression");
        };
        assert_eq!(or_expr.operator, LogicalOperator::Or);
        let Expression::Logical(and_expr) = &or_expr.right else {
            panic!("expected logical-and rhs");
        };
        assert_eq!(and_expr.operator, LogicalOperator::And);
        let Expression::Binary(bit_or_expr) = &and_expr.right else {
            panic!("expected bitwise-or rhs");
        };
        assert_eq!(bit_or_expr.operator, BinaryOperator::BitwiseOr);
        let Expression::Binary(bit_xor_expr) = &bit_or_expr.right else {
            panic!("expected bitwise-xor rhs");
        };
        assert_eq!(bit_xor_expr.operator, BinaryOperator::BitwiseXor);
        let Expression::Binary(bit_and_expr) = &bit_xor_expr.right else {
            panic!("expected bitwise-and rhs");
        };
        assert_eq!(bit_and_expr.operator, BinaryOperator::BitwiseAnd);
    }

    #[test]
    fn parser_pratt_preserves_assignment_associativity() {
        let expression = parse_single_expression("target = left = right;");

        let Expression::Assignment(assignment) = expression else {
            panic!("expected assignment expression");
        };
        assert_eq!(assignment.operator, AssignmentOperator::Assign);
        let Expression::Assignment(nested) = &assignment.right else {
            panic!("expected assignment rhs to associate to the right");
        };
        assert_eq!(nested.operator, AssignmentOperator::Assign);
    }

    #[test]
    fn parser_pratt_preserves_nullish_and_compound_assignment_associativity() {
        let expression = parse_single_expression("a ?? b ?? c;");
        let Expression::Logical(nullish) = expression else {
            panic!("expected nullish-coalescing expression");
        };
        assert_eq!(nullish.operator, LogicalOperator::NullishCoalescing);
        assert!(matches!(
            nullish.left,
            Expression::Logical(ref nested)
                if nested.operator == LogicalOperator::NullishCoalescing
        ));

        let expression = parse_single_expression("a += b *= c;");
        let Expression::Assignment(add_assign) = expression else {
            panic!("expected compound assignment");
        };
        assert_eq!(add_assign.operator, AssignmentOperator::AddAssign);
        let Expression::Assignment(mul_assign) = &add_assign.right else {
            panic!("expected nested rhs assignment");
        };
        assert_eq!(mul_assign.operator, AssignmentOperator::MulAssign);
    }

    #[test]
    fn parser_pratt_preserves_sequence_as_lowest_precedence() {
        let expression = parse_single_expression("a + b << c, d = e = f;");

        let Expression::Sequence(sequence) = expression else {
            panic!("expected sequence expression");
        };
        assert_eq!(sequence.expressions.len(), 2);

        let Expression::Binary(shift_expr) = &sequence.expressions[0] else {
            panic!("expected shift expression in first sequence slot");
        };
        assert_eq!(shift_expr.operator, BinaryOperator::LeftShift);
        assert!(matches!(
            shift_expr.left,
            Expression::Binary(ref add_expr) if add_expr.operator == BinaryOperator::Add
        ));

        let Expression::Assignment(assignment) = &sequence.expressions[1] else {
            panic!("expected assignment expression in second sequence slot");
        };
        let Expression::Assignment(nested) = &assignment.right else {
            panic!("expected assignment rhs to remain right-associative inside sequence");
        };
        assert_eq!(nested.operator, AssignmentOperator::Assign);

        let expression = parse_single_expression("a, b, c;");
        let Expression::Sequence(sequence) = expression else {
            panic!("expected sequence expression");
        };
        assert_eq!(sequence.expressions.len(), 3);
        assert!(matches!(
            sequence.expressions[0],
            Expression::Identifier(ref identifier) if identifier.name == "a"
        ));
        assert!(matches!(
            sequence.expressions[1],
            Expression::Identifier(ref identifier) if identifier.name == "b"
        ));
        assert!(matches!(
            sequence.expressions[2],
            Expression::Identifier(ref identifier) if identifier.name == "c"
        ));
    }

    #[test]
    fn parser_pratt_rejects_unparenthesized_nullish_mixing() {
        assert!(parse("null ?? false || true;").is_err());
        assert!(parse("true && null ?? 5;").is_err());
        assert!(parse("a?.b ?? c || d;").is_err());

        let expression = parse_single_expression("(a?.b ?? c) || d;");
        let Expression::Logical(or_expr) = expression else {
            panic!("expected logical-or expression");
        };
        assert_eq!(or_expr.operator, LogicalOperator::Or);
        assert!(matches!(
            or_expr.left,
            Expression::Logical(ref nullish)
                if nullish.operator == LogicalOperator::NullishCoalescing
        ));

        parse("(null ?? false) || true;").unwrap();
        parse("null ?? (false || true);").unwrap();
        parse("(true && null) ?? 5;").unwrap();
    }

    #[test]
    fn parser_pratt_preserves_optional_chaining_precedence() {
        let expression = parse_single_expression("a?.b.c;");
        let Expression::Member(outer) = expression else {
            panic!("expected member expression");
        };
        assert!(!outer.optional);
        let Expression::Member(inner) = &outer.object else {
            panic!("expected optional member expression");
        };
        assert!(inner.optional);

        let expression = parse_single_expression("a?.();");
        let Expression::Call(optional_call) = expression else {
            panic!("expected call expression");
        };
        assert!(optional_call.optional);
        assert!(matches!(
            optional_call.callee,
            Expression::Identifier(ref identifier) if identifier.name == "a"
        ));

        let expression = parse_single_expression("a?.b()?.c;");
        let Expression::Member(optional_member) = expression else {
            panic!("expected optional member expression");
        };
        assert!(optional_member.optional);
        let Expression::Call(call) = &optional_member.object else {
            panic!("expected call before optional member");
        };
        assert!(!call.optional);
        assert!(matches!(
            call.callee,
            Expression::Member(ref member) if member.optional
        ));

        let expression = parse_single_expression("a?.b + c;");
        let Expression::Binary(add) = expression else {
            panic!("expected additive expression");
        };
        assert_eq!(add.operator, BinaryOperator::Add);
        assert!(matches!(
            add.left,
            Expression::Member(ref member) if member.optional
        ));

        let expression = parse_single_expression("a?.b * c;");
        let Expression::Binary(multiply) = expression else {
            panic!("expected multiplicative expression");
        };
        assert_eq!(multiply.operator, BinaryOperator::Multiply);
        assert!(matches!(
            multiply.left,
            Expression::Member(ref member) if member.optional
        ));

        let expression = parse_single_expression("a?.b ?? c;");
        let Expression::Logical(nullish) = expression else {
            panic!("expected nullish-coalescing expression");
        };
        assert_eq!(nullish.operator, LogicalOperator::NullishCoalescing);
        assert!(matches!(
            nullish.left,
            Expression::Member(ref member) if member.optional
        ));

        let expression = parse_single_expression("a?.b || c;");
        let Expression::Logical(or_expr) = expression else {
            panic!("expected logical-or expression");
        };
        assert_eq!(or_expr.operator, LogicalOperator::Or);
        assert!(matches!(
            or_expr.left,
            Expression::Member(ref member) if member.optional
        ));
    }

    #[test]
    fn parser_pratt_preserves_call_member_and_new_precedence() {
        let expression = parse_single_expression("a.b.c();");
        let Expression::Call(call) = expression else {
            panic!("expected call expression");
        };
        let Expression::Member(callee) = &call.callee else {
            panic!("expected member callee");
        };
        assert!(matches!(callee.object, Expression::Member(_)));

        let expression = parse_single_expression("f()().x;");
        let Expression::Member(member) = expression else {
            panic!("expected member expression");
        };
        assert!(matches!(member.object, Expression::Call(_)));

        let expression = parse_single_expression("new A().b;");
        let Expression::Member(member) = expression else {
            panic!("expected member expression");
        };
        assert!(matches!(member.object, Expression::New(_)));

        let expression = parse_single_expression("new A.b;");
        let Expression::New(new_expr) = expression else {
            panic!("expected new expression");
        };
        assert!(matches!(new_expr.callee, Expression::Member(_)));

        let expression = parse_single_expression("new A()();");
        let Expression::Call(call) = expression else {
            panic!("expected outer call expression");
        };
        assert!(matches!(call.callee, Expression::New(_)));

        let expression = parse_single_expression("new new A();");
        let Expression::New(outer_new) = expression else {
            panic!("expected outer new expression");
        };
        assert!(matches!(outer_new.callee, Expression::New(_)));
    }

    #[test]
    fn parser_pratt_handles_arrow_ambiguity() {
        let expression = parse_single_expression("(a) => a + 1;");
        let Expression::ArrowFunction(arrow) = expression else {
            panic!("expected arrow function");
        };
        assert_eq!(arrow.params.len(), 1);
        assert!(matches!(
            arrow.body,
            ArrowBody::Expression(ref body)
                if matches!(body.as_ref(), Expression::Binary(expr) if expr.operator == BinaryOperator::Add)
        ));

        let expression = parse_single_expression("(a, b) => a + b;");
        let Expression::ArrowFunction(arrow) = expression else {
            panic!("expected arrow function");
        };
        assert_eq!(arrow.params.len(), 2);

        let expression = parse_single_expression("(a + b);");
        assert!(matches!(
            expression,
            Expression::Binary(ref expr) if expr.operator == BinaryOperator::Add
        ));

        let expression = parse_single_expression("(a, b + c);");
        let Expression::Sequence(sequence) = expression else {
            panic!("expected sequence expression");
        };
        assert_eq!(sequence.expressions.len(), 2);
        assert!(matches!(
            sequence.expressions[1],
            Expression::Binary(ref expr) if expr.operator == BinaryOperator::Add
        ));
    }

    #[test]
    fn parser_pratt_distinguishes_object_literals_from_assignment_patterns() {
        let expression = parse_single_expression("({ a: b });");
        let Expression::Object(object) = expression else {
            panic!("expected object expression");
        };
        assert_eq!(object.properties.len(), 1);
        assert!(matches!(
            object.properties[0],
            ObjectProperty::Property { ref value, .. }
                if matches!(value, Expression::Identifier(identifier) if identifier.name == "b")
        ));

        let expression = parse_single_expression("({ a: b } = obj);");
        let Expression::Assignment(assignment) = expression else {
            panic!("expected assignment expression");
        };
        assert!(matches!(assignment.left, Expression::Object(_)));
        assert!(matches!(
            assignment.right,
            Expression::Identifier(ref identifier) if identifier.name == "obj"
        ));

        let expression = parse_single_expression("[a, b] = arr;");
        let Expression::Assignment(assignment) = expression else {
            panic!("expected assignment expression");
        };
        assert!(matches!(assignment.left, Expression::Array(_)));
    }

    #[test]
    fn parser_pratt_handles_exponentiation_edge_cases() {
        assert!(parse("-2 ** 2;").is_err());

        let expression = parse_single_expression("(-2) ** 2;");
        let Expression::Binary(exponentiate) = expression else {
            panic!("expected exponentiation expression");
        };
        assert_eq!(exponentiate.operator, BinaryOperator::Exponentiate);
        assert!(matches!(exponentiate.left, Expression::Unary(_)));

        let expression = parse_single_expression("-(2 ** 2 ** 3);");
        let Expression::Unary(unary) = expression else {
            panic!("expected unary expression");
        };
        assert_eq!(unary.operator, UnaryOperator::Negative);
        let Expression::Binary(exponentiate) = &unary.argument else {
            panic!("expected exponentiation under unary");
        };
        assert_eq!(exponentiate.operator, BinaryOperator::Exponentiate);
        assert!(matches!(
            exponentiate.right,
            Expression::Binary(ref nested) if nested.operator == BinaryOperator::Exponentiate
        ));
    }

    #[test]
    fn parser_pratt_handles_in_operator_contexts() {
        let expression = parse_single_expression("\"a\" in obj;");
        let Expression::Binary(binary) = expression else {
            panic!("expected binary expression");
        };
        assert_eq!(binary.operator, BinaryOperator::In);

        parse("for (a in b) {}").unwrap();
        assert!(parse("for (a = 1 in b;;) {}").is_err());
    }

    #[test]
    fn parser_pratt_handles_complex_grouped_expression_stress_case() {
        parse("((a?.b.c(d, e?.f + g * h ** i)?.j[k]) ?? l) || m && n ? x : y = z;").unwrap();
    }

    #[test]
    fn ast_to_js_round_trips_script_programs() {
        let source = r#"
const makeBox = (value, extras = [seed, ...rest]) => ({ value, extras });
class Box extends Base {
  static count = 1;
  #value = 0;
  method(x) { return this.#value + x; }
}
({ value: target, nested: [first, ...rest] } = sourceValue);
new Box().method(1);
"#;

        let program = parse(source).unwrap();
        let emitted = program_to_js(&program);
        let reparsed = parse(&emitted).unwrap();
        let re_emitted = program_to_js(&reparsed);
        assert_eq!(emitted, re_emitted);
    }

    #[test]
    fn ast_to_js_round_trips_module_programs() {
        let source = r#"
import data, { named as alias } from "pkg" with { type: "json" };
export { alias as named };
export default class Box {
  value = 1;
}
"#;

        let options = ParseOptions {
            is_module: true,
            is_strict: true,
        };
        let program = parse_with_options(source, options).unwrap();
        let emitted = program_to_js(&program);
        let reparsed = parse_with_options(&emitted, options).unwrap();
        let re_emitted = program_to_js(&reparsed);
        println!("js:\n{re_emitted}");
        assert_eq!(emitted, re_emitted);
    }

    #[test]
    fn ast_to_js_preserves_object_literal_accessors() {
        let program =
            parse("({ get value() { return inner; }, set value(next) { inner = next; } });")
                .unwrap();
        let [Statement::Expression(statement)] = program.body.as_slice() else {
            panic!("expected a single expression statement");
        };
        let Expression::Object(object) = &statement.expression else {
            panic!("expected object literal");
        };
        assert!(matches!(
            object.properties[0],
            ObjectProperty::Property {
                kind: ObjectPropertyKind::Getter,
                ..
            }
        ));
        assert!(matches!(
            object.properties[1],
            ObjectProperty::Property {
                kind: ObjectPropertyKind::Setter,
                ..
            }
        ));

        let emitted = program_to_js(&program);
        assert!(emitted.contains("get value()"));
        assert!(emitted.contains("set value(next)"));

        let reparsed = parse(&emitted).unwrap();
        assert_eq!(program_to_js(&reparsed), emitted);
    }

    #[test]
    fn program_to_js_does_not_wrap_plain_expression_statements() {
        let emitted = program_to_js(&parse("verifyProperty(obj, \"x\", {});").unwrap());
        assert_eq!(emitted, "verifyProperty(obj, \"x\", {  });");
    }

    #[test]
    fn program_to_js_keeps_object_expression_statements_parenthesized() {
        let emitted = program_to_js(&parse("({ value: 1 });").unwrap());
        assert_eq!(emitted, "({ value: 1 });");
    }

    #[test]
    fn program_to_js_preserves_linebreak_let_in_statement_position() {
        let program = parse("if (false) let\nx = 1;").unwrap();
        let emitted = program_to_js(&program);
        assert_eq!(emitted, "if (false)\n  let\n    x = 1;");

        let reparsed = parse(&emitted).unwrap();
        let [Statement::If(statement)] = reparsed.body.as_slice() else {
            panic!("expected a single if statement");
        };
        assert!(matches!(
            statement.consequent.as_ref(),
            Statement::VariableDeclaration(VariableDeclaration {
                kind: VariableKind::Let,
                ..
            })
        ));
    }

    #[test]
    fn program_to_js_preserves_linebreak_let_in_labeled_statement_position() {
        let program = parse("if (false) { L: let\n{} }").unwrap();
        let emitted = program_to_js(&program);
        assert_eq!(emitted, "if (false) {\n  L:\n    let\n      {  };\n}");

        let reparsed = parse(&emitted).unwrap();
        let [Statement::If(statement)] = reparsed.body.as_slice() else {
            panic!("expected a single if statement");
        };
        let Statement::Block(block) = statement.consequent.as_ref() else {
            panic!("expected block consequent");
        };
        assert!(matches!(
            block.body.as_slice(),
            [Statement::Labeled(LabeledStatement { body, .. })]
                if matches!(
                    body.as_ref(),
                    Statement::VariableDeclaration(VariableDeclaration {
                        kind: VariableKind::Let,
                        ..
                    })
                )
        ));
    }

    #[test]
    fn parser_and_emitter_preserve_member_access_on_new_results() {
        let expression = parse_single_expression("new Function(\"x\").apply;");
        let Expression::Member(member) = &expression else {
            panic!("expected member expression, got: {expression:#?}");
        };
        assert!(matches!(member.object, Expression::New(_)));

        let emitted = expression_to_js(&expression);
        assert_eq!(emitted, "(new Function(\"x\")).apply");
    }

    #[test]
    fn program_to_js_preserves_member_access_on_new_results_in_initializers() {
        let emitted = program_to_js(&parse("var obj = new Function(\"x\").apply;").unwrap());
        assert_eq!(emitted, "var obj = (new Function(\"x\")).apply;");
    }

    #[test]
    fn program_to_js_preserves_parenthesized_new_callees_rooted_in_calls() {
        let emitted = program_to_js(&parse("var obj = new (Function(\"x\").apply);").unwrap());
        assert_eq!(emitted, "var obj = new (Function(\"x\").apply);");

        let emitted = program_to_js(
            &parse("Object.seal(new (Object.getPrototypeOf(() => {}).constructor)());").unwrap(),
        );
        assert_eq!(program_to_js(&parse(&emitted).unwrap()), emitted);
    }

    #[test]
    fn parser_handles_private_in_and_using_declarations() {
        parse_with_options(
            "class C { #x; static has(value) { return #x in value; } } await using resource = null;",
            ParseOptions {
                is_module: true,
                is_strict: true,
            },
        )
        .unwrap();
        parse("for (using of of [0, 1, 2]) { break; }").unwrap();
        parse("for ({ x = 1, nested: [value] } of values) { break; }").unwrap();
    }

    #[test]
    fn parser_rejects_for_in_initializer_expression() {
        assert!(parse("for (a = 0 in {});").is_err());
    }

    #[test]
    fn parser_handles_async_function_await_name_and_auto_accessor_fields() {
        parse("async function await() { return 1; }").unwrap();
        parse("var C = class { accessor $; accessor _ = 1; };").unwrap();
        parse("const value = true ?.30 : false;").unwrap();
    }

    #[test]
    fn parser_handles_unicode_identifiers_and_legacy_contextual_names() {
        parse("var let = 1; var object = { let };").unwrap();
        parse("function* g() { (function yield() {}); }").unwrap();
        parse("var \\u088F; var _\\u0AFD; var ゛; var \\u309B;").unwrap();
    }

    #[test]
    fn parser_handles_let_identifier_and_decorated_classes() {
        parse("let = 1;").unwrap();
        parse("for (let in obj) ;").unwrap();
        parse("async function f() { await using let = 1; }").unwrap_err();
        parse("@dec class C { @logged method() {} @memo accessor field; }").unwrap();
    }

    #[test]
    fn parser_handles_debugger_and_annex_b_for_in_initializer() {
        parse("debugger; for (var a = 0 in {});").unwrap();
    }

    #[test]
    fn parser_handles_division_after_brace_terminated_expressions() {
        parse("({ valueOf: function() { return 1; } } / 1);").unwrap();
        parse("(function() { return 1; } / {});").unwrap();
        parse("({ [Symbol.toPrimitive]: function() { return 2n; } } / 2n);").unwrap();
        parse(
            "assert.sameValue({ [Symbol.toPrimitive]: function() { return 2n; } } / 2n, 1n, 'ok');",
        )
        .unwrap();
    }

    #[test]
    fn parser_handles_invalid_escapes_only_in_tagged_templates() {
        parse("tag`\\xg`;").unwrap();
        assert!(parse("`\\xg`;").is_err());
        parse("`\\u{10ffff}`;").unwrap();
    }

    #[test]
    fn parser_rejects_invalid_assignment_and_update_targets() {
        assert!(parse("x + y = 1;").is_err());
        assert!(parse("\"use strict\"; import('')++;").is_err());
    }

    #[test]
    fn parser_rejects_strict_mode_early_errors() {
        assert!(parse("\"use strict\"; with ({}) {}").is_err());
        assert!(parse("\"use strict\"; var eval;").is_err());
        assert!(parse("\"use strict\"; for (var a = 0 in {});").is_err());
        assert!(
            parse_with_options(
                "with ({}) {}",
                ParseOptions {
                    is_module: false,
                    is_strict: true,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn parser_rejects_declarations_in_statement_position() {
        parse("if (false) let\nx = 1;").unwrap();
        assert!(parse("while (false) const x = null;").is_err());
        assert!(parse("\"use strict\"; if (flag) function f() {}").is_err());
        parse("label: function f() {}").unwrap();
    }

    #[test]
    fn parser_rejects_escaped_dynamic_import_keyword() {
        assert!(parse("im\\u0070ort('./x.js');").is_err());
    }

    #[test]
    fn parser_handles_sloppy_yield_division() {
        parse("var yield = 12, a = 3, b = 6, g = 2; yield /a; b/g;").unwrap();
    }

    #[test]
    fn parser_handles_class_field_await_identifier_in_script_async_function() {
        parse("var await = 1; async function getClass() { return class { x = await; }; }").unwrap();
        assert!(
            parse_with_options(
                "async () => class { x = await };",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
    }
    #[test]
    fn parser_simple() {
        println!("{:#?}", parse("a+b").unwrap());
    }
    #[test]
    fn parser_rejects_invalid_dynamic_import_forms() {
        assert!(parse("typeof import;").is_err());
        assert!(parse("import();").is_err());
        assert!(parse("import('./x.js', {}, 'extra');").is_err());
        assert!(parse("new import('./x.js');").is_err());
        assert!(parse("import.source();").is_err());
        assert!(parse("import.defer(...['./x.js']);").is_err());
    }

    #[test]
    fn parser_rejects_class_private_name_and_field_early_errors() {
        assert!(parse("class C { #x; #x; }").is_err());
        assert!(parse("class C { field = arguments; }").is_err());
        assert!(parse("class C extends B { field = () => super(); }").is_err());
        assert!(parse("class C { constructor; }").is_err());
        assert!(parse("class C { static prototype; }").is_err());
        assert!(parse("class C { method() { this.#x; } }").is_err());
        assert!(parse("class C extends B { #x() {} method() { super.#x(); } }").is_err());
        assert!(parse("class C { #x; method() { delete this.#x; } }").is_err());
    }

    #[test]
    fn parser_rejects_escaped_reserved_words_and_nested_module_declarations() {
        assert!(parse("var \\u{62}\\u{72}\\u{65}\\u{61}\\u{6b} = 123;").is_err());
        assert!(parse("var typeo\\u0066 = 123;").is_err());
        assert!(parse("var typeo\\u{66} = 123;").is_err());
        assert!(parse("var x = { typ\\u0065of } = { typeof: 42 };").is_err());
        assert!(parse("var x = ({ typ\\u0065of }) => {};").is_err());
        assert!(parse("({ \\u0061sync* m() {} });").is_err());
        assert!(
            parse_with_options(
                "{ import value from './dep.js'; }",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "export { value };",
                ParseOptions {
                    is_module: false,
                    is_strict: false,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "<!--",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "-->",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "/*\n*/-->",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn parser_rejects_module_and_control_flow_early_errors() {
        assert!(parse("new.target;").is_err());
        assert!(
            parse_with_options(
                "new.target;",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(parse("break;").is_err());
        assert!(parse("continue outer;").is_err());
        assert!(parse("try {} catch ({ x }) { let x; }").is_err());
        assert!(parse("class C { static { return; } }").is_err());
    }

    #[test]
    fn parser_rejects_ill_formed_module_export_names_and_strict_template_substitutions() {
        assert!(
            parse_with_options(
                "export { \"\\uD83D\" } from './dep.js';",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "export { \"ok\" as \"\\uD83D\" } from './dep.js';",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "import { \"\\uD83D\" as foo } from './dep.js';",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "export { Foo as \"\\uD83D\" }; function Foo() {}",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "export { \"\\uD83D\\uDE00\" as smile } from './dep.js';",
                ParseOptions {
                    is_module: true,
                    is_strict: true,
                },
            )
            .is_ok()
        );
        assert!(parse("`${'\\07'}`;").is_ok());
        assert!(
            parse_with_options(
                "`${'\\07'}`;",
                ParseOptions {
                    is_module: false,
                    is_strict: true,
                },
            )
            .is_err()
        );
        assert!(
            parse_with_options(
                "tag`${'\\07'}`;",
                ParseOptions {
                    is_module: false,
                    is_strict: true,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn parser_rejects_invalid_numeric_separators_and_bigints() {
        assert!(parse("1_;").is_err());
        assert!(parse("1__0;").is_err());
        assert!(parse("1e_1;").is_err());
        assert!(parse("0_1n;").is_err());
        assert!(parse("08n;").is_err());
        parse("0x1n + 0b10n + 0o7n;").unwrap();
    }

    #[test]
    fn parser_handles_unicode_regexp_literals_in_full_source() {
        let tokens = lex("const r = /\u{20BB7}/u;").unwrap();
        let regexp_body = tokens
            .iter()
            .find_map(|token| match &token.kind {
                TokenKind::RegExp { body, .. } => Some(body.as_str()),
                _ => None,
            })
            .unwrap();
        assert_eq!(regexp_body, "\u{20BB7}");
        assert!(parse("const r = /\u{20BB7}/u;").is_ok());
        assert!(parse("const r = /\u{20BB7}/v;").is_ok());
    }

    #[test]
    fn parser_handles_regexp_builtin_exec_unicode_fixture() {
        let source = include_str!(
            "../../../test262/test/built-ins/RegExp/prototype/exec/regexp-builtin-exec-v-u-flag.js"
        );
        let tokens = lex(source).unwrap();
        let regexp_bodies: Vec<_> = tokens
            .iter()
            .filter_map(|token| match &token.kind {
                TokenKind::RegExp { body, flags } => Some((body.clone(), flags.clone())),
                _ => None,
            })
            .collect();
        assert!(
            regexp_bodies
                .iter()
                .any(|(body, flags)| body == "\u{20BB7}" && flags == "u")
        );
        assert!(
            regexp_bodies
                .iter()
                .any(|(body, flags)| body == "\u{20BB7}" && flags == "v")
        );
        for (body, flags) in &regexp_bodies {
            let parsed = parse_regexp_pattern(body, flags);
            assert!(
                parsed.is_ok(),
                "body={body:?} flags={flags:?} parsed={parsed:?}"
            );
        }
        let parsed = parse(source);
        assert!(parsed.is_ok(), "{parsed:?}");
    }
}
