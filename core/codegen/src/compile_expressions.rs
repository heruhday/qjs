use super::*;

impl Codegen {
    pub(crate) fn compile_expression(
        &mut self,
        expression: &Expression,
    ) -> Result<u8, CodegenError> {
        match expression {
            Expression::Identifier(identifier) => self.compile_identifier_value(identifier),
            Expression::Literal(literal) => self.compile_literal(literal),
            Expression::This(_) => self.compile_this(expression.span()),
            Expression::Array(array) => self.compile_array_expression(array),
            Expression::Object(object) => self.compile_object_expression(object),
            Expression::Function(function) => self.compile_function_expression(function),
            Expression::ArrowFunction(function) => self.compile_arrow_function_expression(function),
            Expression::Unary(unary) => self.compile_unary_expression(unary),
            Expression::Update(update) => self.compile_update_expression(update),
            Expression::Binary(binary) => self.compile_binary_expression(binary),
            Expression::Logical(logical) => self.compile_logical_expression(logical),
            Expression::Assignment(assignment) => self.compile_assignment_expression(assignment),
            Expression::Conditional(conditional) => {
                self.compile_conditional_expression(conditional)
            }
            Expression::Sequence(sequence) => self.compile_sequence_expression(sequence),
            Expression::Call(call) => self.compile_call_expression(call),
            Expression::Member(member) => self.compile_member_expression(member),
            Expression::New(new) => self.compile_new_expression(new),
            Expression::Super(span) => Err(CodegenError::Unsupported {
                feature: "super",
                span: *span,
            }),
            Expression::PrivateIdentifier(identifier) => {
                self.load_runtime_atom(&identifier.name, identifier.span)
            }
            Expression::Class(class) => Err(CodegenError::Unsupported {
                feature: "class expressions",
                span: class.span,
            }),
            Expression::TaggedTemplate(node) => self.compile_tagged_template_expression(node),
            Expression::MetaProperty(node) => Err(CodegenError::Unsupported {
                feature: "meta properties",
                span: node.span,
            }),
            Expression::Yield(node) => Err(CodegenError::Unsupported {
                feature: "yield",
                span: node.span,
            }),
            Expression::Await(node) => self.compile_await_expression(node),
        }
    }

    pub(crate) fn compile_await_expression(
        &mut self,
        expression: &ast::AwaitExpression,
    ) -> Result<u8, CodegenError> {
        // Compile the argument expression to get the value/Promise
        let arg_reg = self.compile_expression(&expression.argument)?;
        
        // For proper await semantics, we would need to:
        // 1. Detect if it's a Promise
        // 2. Suspend execution and attach a continuation
        // 3. Resume when the Promise settles
        //
        // The current VM architecture doesn't support frame suspension/resumption,
        // so we emit the Await opcode which currently just passes through the value.
        // This allows await syntax to parse and run, though full Promise unwrapping
        // would require transforming async functions into state machines.
        self.builder.emit_await(arg_reg);
        self.builder.emit_mov(arg_reg, ACC);
        Ok(arg_reg)
    }

    pub(crate) fn compile_identifier_value(
        &mut self,
        identifier: &Identifier,
    ) -> Result<u8, CodegenError> {
        if let Some(home_reg) = self.fast_name_reg(&identifier.name) {
            let reg = self.alloc_temp(Some(identifier.span))?;
            self.builder.emit_mov(reg, home_reg);
            return Ok(reg);
        }
        self.compile_identifier_current(identifier)
    }

    pub(crate) fn compile_identifier_current(
        &mut self,
        identifier: &Identifier,
    ) -> Result<u8, CodegenError> {
        if let Some(reg) = self.fast_name_reg(&identifier.name) {
            self.temp_top = self.temp_top.max(reg);
            return Ok(reg);
        }
        if let Some((name, slot)) = self.current_self_upvalue.clone()
            && identifier.name == name
        {
            let reg = if let Some(reg) = self.current_self_upvalue_reg {
                reg
            } else {
                let reg = self.alloc_temp(Some(identifier.span))?;
                self.builder.emit_get_upval(reg, slot);
                self.current_self_upvalue_reg = Some(reg);
                reg
            };
            self.temp_top = self.temp_top.max(reg);
            return Ok(reg);
        }
        let reg = self.alloc_temp(Some(identifier.span))?;
        let slot = self.name_slot(&identifier.name)?;
        self.builder.emit_load_name(reg, slot);
        Ok(reg)
    }

    pub(crate) fn compile_readonly_expression(
        &mut self,
        expression: &Expression,
    ) -> Result<u8, CodegenError> {
        match expression {
            Expression::Identifier(identifier) => self.compile_identifier_current(identifier),
            _ => self.compile_expression(expression),
        }
    }

    pub(crate) fn compile_this(&mut self, span: Span) -> Result<u8, CodegenError> {
        let reg = self.alloc_temp(Some(span))?;
        self.builder.emit_load_this();
        self.builder.emit_mov(reg, ACC);
        Ok(reg)
    }

    pub(crate) fn compile_literal(&mut self, literal: &Literal) -> Result<u8, CodegenError> {
        match literal {
            Literal::Null(span) => {
                let reg = self.alloc_temp(Some(*span))?;
                self.builder.emit_load_null();
                self.builder.emit_mov(reg, ACC);
                Ok(reg)
            }
            Literal::Boolean(node) => {
                let reg = self.alloc_temp(Some(node.span))?;
                if node.value {
                    self.builder.emit_load_true(reg);
                } else {
                    self.builder.emit_load_false(reg);
                }
                Ok(reg)
            }
            Literal::Number(node) => {
                let reg = self.alloc_temp(Some(node.span))?;
                let value = parse_number_literal(node)?;
                if value.fract() == 0.0 && value >= i16::MIN as f64 && value <= i16::MAX as f64 {
                    self.builder.emit_load_i(reg, value as i16);
                } else {
                    let index = self.builder.add_constant(make_number(value));
                    self.builder.emit_load_k(reg, index);
                }
                Ok(reg)
            }
            Literal::String(node) => self.load_runtime_string(&node.value, node.span),
            Literal::Template(node) => self.compile_template_literal(node),
            Literal::RegExp(node) => Err(CodegenError::Unsupported {
                feature: "regexp literals",
                span: node.span,
            }),
        }
    }

    pub(crate) fn emit_literal_to_acc(&mut self, literal: &Literal) -> Result<bool, CodegenError> {
        match literal {
            Literal::Null(_) => {
                self.builder.emit_load_null();
                Ok(true)
            }
            Literal::Boolean(node) => {
                if node.value {
                    self.builder.emit_load_true(ACC);
                } else {
                    self.builder.emit_load_false(ACC);
                }
                Ok(true)
            }
            Literal::Number(node) => {
                let value = parse_number_literal(node)?;
                if value.fract() == 0.0 && value >= i16::MIN as f64 && value <= i16::MAX as f64 {
                    self.builder.emit_load_i(ACC, value as i16);
                } else {
                    let index = self.builder.add_constant(make_number(value));
                    self.builder.emit_load_k(ACC, index);
                }
                Ok(true)
            }
            Literal::String(node) => {
                let index = self.builder.add_constant(make_undefined());
                self.string_constants.push((index, node.value.to_owned()));
                self.builder.emit_load_k(ACC, index);
                Ok(true)
            }
            Literal::Template(_) | Literal::RegExp(_) => Ok(false),
        }
    }

    pub(crate) fn emit_undefined_to_acc(&mut self) {
        let index = *self
            .undefined_const
            .get_or_insert_with(|| self.builder.add_constant(make_undefined()));
        self.builder.emit_load_k(ACC, index);
    }

    pub(crate) fn compile_template_literal(
        &mut self,
        literal: &ast::TemplateLiteral,
    ) -> Result<u8, CodegenError> {
        let parts = parse_template_literal_parts(literal)?;
        let result = self.load_runtime_string("", literal.span)?;

        for part in parts {
            let value = match part {
                TemplatePart::Text(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    self.load_runtime_string(&text, literal.span)?
                }
                TemplatePart::Expression(expression) => self.compile_expression(&expression)?,
            };
            self.builder.emit_add(result, value);
            self.builder.emit_mov(result, ACC);
            self.temp_top = result;
        }

        Ok(result)
    }

    fn compile_string_array(&mut self, values: &[String], span: Span) -> Result<u8, CodegenError> {
        let array_reg = self.alloc_temp(Some(span))?;
        self.builder
            .emit_new_arr(array_reg, values.len().min(u8::MAX as usize) as u8);

        for value in values {
            let value_reg = self.load_runtime_string(value, span)?;
            if value_reg != ACC {
                self.builder.emit_mov(ACC, value_reg);
            }
            self.builder.emit_array_push_acc(array_reg);
            self.temp_top = array_reg;
        }

        Ok(array_reg)
    }

    fn compile_optional_string_array(
        &mut self,
        values: &[Option<String>],
        span: Span,
    ) -> Result<u8, CodegenError> {
        let array_reg = self.alloc_temp(Some(span))?;
        self.builder
            .emit_new_arr(array_reg, values.len().min(u8::MAX as usize) as u8);

        for value in values {
            let value_reg = match value {
                Some(text) => self.load_runtime_string(text, span)?,
                None => self.load_undefined(Some(span))?,
            };
            if value_reg != ACC {
                self.builder.emit_mov(ACC, value_reg);
            }
            self.builder.emit_array_push_acc(array_reg);
            self.temp_top = array_reg;
        }

        Ok(array_reg)
    }

    fn compile_tagged_template_object(
        &mut self,
        quasi: &ast::TemplateLiteral,
        quasis: &[TemplateQuasiPart],
    ) -> Result<u8, CodegenError> {
        let helper_reg = self.alloc_temp(Some(quasi.span))?;
        let helper_slot = self.name_slot("__qjs_get_template_object")?;
        self.builder.emit_load_name(helper_reg, helper_slot);

        let key = format!(
            "{}:{}:{}:{}:{}",
            quasi.span.start.line,
            quasi.span.start.column,
            quasi.span.end.line,
            quasi.span.end.column,
            quasi.raw
        );
        let key_reg = self.load_runtime_string(&key, quasi.span)?;
        let cooked_values = quasis
            .iter()
            .map(|part| part.cooked.clone())
            .collect::<Vec<_>>();
        let raw_values = quasis
            .iter()
            .map(|part| part.raw.clone())
            .collect::<Vec<_>>();
        let cooked_reg = self.compile_optional_string_array(&cooked_values, quasi.span)?;
        let raw_reg = self.compile_string_array(&raw_values, quasi.span)?;

        for (offset, reg) in [key_reg, cooked_reg, raw_reg].into_iter().enumerate() {
            let expected = helper_reg as usize + 1 + offset;
            if expected >= ACC as usize {
                return Err(CodegenError::RegisterOverflow {
                    span: Some(quasi.span),
                });
            }
            if reg != expected as u8 {
                self.builder.emit_mov(expected as u8, reg);
                self.temp_top = self.temp_top.max(expected as u8);
            }
        }

        self.builder.emit_call(helper_reg, 3);
        self.builder.emit_mov(helper_reg, ACC);
        self.temp_top = helper_reg;
        Ok(helper_reg)
    }

    fn compile_member_callee(
        &mut self,
        member: &MemberExpression,
    ) -> Result<(u8, u8), CodegenError> {
        let this_reg = self.compile_expression(&member.object)?;
        let callee_reg = self.alloc_temp(Some(member.span))?;

        match &member.property {
            MemberProperty::Identifier(identifier) => {
                let slot = self.property_slot(&identifier.name)?;
                self.builder.emit_get_prop(callee_reg, this_reg, slot);
            }
            MemberProperty::Computed { expression, .. } => {
                let key_reg = self.compile_expression(expression)?;
                self.builder.emit_get_prop_acc(this_reg, key_reg);
                self.builder.emit_mov(callee_reg, ACC);
                self.temp_top = callee_reg;
            }
            MemberProperty::PrivateName(identifier) => {
                return Err(CodegenError::Unsupported {
                    feature: "private method calls",
                    span: identifier.span,
                });
            }
        }

        Ok((callee_reg, this_reg))
    }

    pub(crate) fn compile_tagged_template_expression(
        &mut self,
        expression: &ast::TaggedTemplateExpression,
    ) -> Result<u8, CodegenError> {
        let (quasis, values) = parse_tagged_template_literal_parts(&expression.quasi)?;

        if let Expression::Member(member) = &expression.tag {
            let (callee_reg, this_reg) = self.compile_member_callee(member.as_ref())?;
            let template_reg = self.compile_tagged_template_object(&expression.quasi, &quasis)?;
            let arg_regs = std::iter::once(Ok(template_reg))
                .chain(values.iter().map(|value| self.compile_expression(value)))
                .collect::<Result<Vec<_>, _>>()?;

            for (index, reg) in arg_regs.into_iter().enumerate() {
                let expected = callee_reg as usize + 1 + index;
                if expected >= ACC as usize {
                    return Err(CodegenError::RegisterOverflow {
                        span: Some(expression.span),
                    });
                }
                if reg != expected as u8 {
                    self.builder.emit_mov(expected as u8, reg);
                    self.temp_top = self.temp_top.max(expected as u8);
                }
            }

            self.builder
                .emit_call_this(callee_reg, this_reg, (values.len() + 1) as u8);
            self.builder.emit_mov(callee_reg, ACC);
            self.temp_top = callee_reg;
            return Ok(callee_reg);
        }

        let callee_reg = self.compile_expression(&expression.tag)?;
        let template_reg = self.compile_tagged_template_object(&expression.quasi, &quasis)?;
        let arg_regs = std::iter::once(Ok(template_reg))
            .chain(values.iter().map(|value| self.compile_expression(value)))
            .collect::<Result<Vec<_>, _>>()?;

        for (index, reg) in arg_regs.into_iter().enumerate() {
            let expected = callee_reg as usize + 1 + index;
            if expected >= ACC as usize {
                return Err(CodegenError::RegisterOverflow {
                    span: Some(expression.span),
                });
            }
            if reg != expected as u8 {
                self.builder.emit_mov(expected as u8, reg);
                self.temp_top = self.temp_top.max(expected as u8);
            }
        }

        self.builder
            .emit_call(callee_reg, (values.len() + 1).min(u8::MAX as usize) as u8);
        let result = if self.current_self_upvalue_reg == Some(callee_reg) {
            self.alloc_temp(Some(expression.span))?
        } else {
            callee_reg
        };
        self.builder.emit_mov(result, ACC);
        self.temp_top = result;
        Ok(result)
    }

    pub(crate) fn compile_array_expression(
        &mut self,
        expression: &ast::ArrayExpression,
    ) -> Result<u8, CodegenError> {
        let array_reg = self.alloc_temp(Some(expression.span))?;
        self.builder.emit_new_arr(
            array_reg,
            expression.elements.len().min(u8::MAX as usize) as u8,
        );

        for element in &expression.elements {
            match element {
                Some(ast::ArrayElement::Expression(expression)) => {
                    if let Expression::Literal(literal) = expression
                        && self.emit_literal_to_acc(literal)?
                    {
                        self.builder.emit_array_push_acc(array_reg);
                        self.temp_top = array_reg;
                        continue;
                    }
                    let value = self.compile_expression(expression)?;
                    if value != ACC {
                        self.builder.emit_mov(ACC, value);
                    }
                    self.builder.emit_array_push_acc(array_reg);
                    self.temp_top = array_reg;
                }
                Some(ast::ArrayElement::Spread { argument, .. }) => {
                    let source = self.compile_expression(argument)?;
                    self.builder.emit_spread(array_reg, source);
                    self.temp_top = array_reg;
                }
                None => {
                    self.emit_undefined_to_acc();
                    self.builder.emit_array_push_acc(array_reg);
                    self.temp_top = array_reg;
                }
            }
        }

        Ok(array_reg)
    }

    pub(crate) fn compile_object_expression(
        &mut self,
        expression: &ObjectExpression,
    ) -> Result<u8, CodegenError> {
        let object_reg = self.alloc_temp(Some(expression.span))?;
        self.builder.emit_new_obj(object_reg);

        for property in &expression.properties {
            match property {
                ObjectProperty::Spread { span, .. } => {
                    return Err(CodegenError::Unsupported {
                        feature: "object spread",
                        span: *span,
                    });
                }
                ObjectProperty::Property {
                    key,
                    value,
                    kind,
                    span,
                    ..
                } => {
                    if !matches!(kind, ObjectPropertyKind::Init | ObjectPropertyKind::Method) {
                        return Err(CodegenError::Unsupported {
                            feature: "getters/setters in object literals",
                            span: *span,
                        });
                    }
                    let value_reg = self.compile_expression(value)?;
                    self.emit_store_property(object_reg, key, value_reg)?;
                    self.temp_top = object_reg;
                }
            }
        }

        Ok(object_reg)
    }

    pub(crate) fn compile_function_expression(
        &mut self,
        function: &Function,
    ) -> Result<u8, CodegenError> {
        let reg = self.alloc_temp(Some(function.span))?;
        let const_index = self.reserve_function_constant();
        self.pending_functions.push_back(PendingFunction {
            const_index,
            body: PendingFunctionBody::Function(function.clone()),
        });
        self.builder.emit_new_func(reg, const_index);
        if function.id.is_some() {
            self.builder.emit_set_upval(reg, 0);
        }
        Ok(reg)
    }

    pub(crate) fn compile_arrow_function_expression(
        &mut self,
        function: &ArrowFunction,
    ) -> Result<u8, CodegenError> {
        let reg = self.alloc_temp(Some(function.span))?;
        let const_index = self.reserve_function_constant();
        self.pending_functions.push_back(PendingFunction {
            const_index,
            body: PendingFunctionBody::Arrow(function.clone()),
        });
        self.builder.emit_new_func(reg, const_index);
        Ok(reg)
    }

    pub(crate) fn compile_unary_expression(
        &mut self,
        expression: &UnaryExpression,
    ) -> Result<u8, CodegenError> {
        match expression.operator {
            UnaryOperator::Typeof => match &expression.argument {
                Expression::Identifier(identifier) => {
                    if self.fast_name_reg(&identifier.name).is_some()
                        || self
                            .current_self_upvalue
                            .as_ref()
                            .is_some_and(|(name, _)| identifier.name == *name)
                    {
                        let reg = self.compile_identifier_current(identifier)?;
                        self.builder.emit_typeof(reg, reg);
                        return Ok(reg);
                    }
                    let reg = self.alloc_temp(Some(expression.span))?;
                    let slot = self.name_slot(&identifier.name)?;
                    self.builder.emit_typeof_name(reg, slot);
                    Ok(reg)
                }
                argument => {
                    let reg = self.compile_expression(argument)?;
                    self.builder.emit_typeof(reg, reg);
                    Ok(reg)
                }
            },
            UnaryOperator::Positive => {
                let reg = self.compile_expression(&expression.argument)?;
                self.builder.emit_to_num(reg, reg);
                Ok(reg)
            }
            UnaryOperator::Negative => {
                let reg = self.compile_expression(&expression.argument)?;
                self.builder.emit_neg(reg);
                self.builder.emit_mov(reg, ACC);
                Ok(reg)
            }
            UnaryOperator::BitNot => {
                let reg = self.compile_expression(&expression.argument)?;
                self.builder.emit_bit_not(reg);
                self.builder.emit_mov(reg, ACC);
                Ok(reg)
            }
            UnaryOperator::LogicalNot => {
                let reg = self.compile_expression(&expression.argument)?;
                let false_branch = self.emit_placeholder_jmp_false(reg);
                self.builder.emit_load_false(reg);
                let end_jump = self.emit_placeholder_jmp();
                let true_start = self.builder.len();
                self.patch_jump(false_branch, true_start, JumpPatchKind::JmpFalse { reg });
                self.builder.emit_load_true(reg);
                let end = self.builder.len();
                self.patch_jump(end_jump, end, JumpPatchKind::Jmp);
                Ok(reg)
            }
            UnaryOperator::Void => {
                let reg = self.compile_expression(&expression.argument)?;
                let target = reg;
                let undef = self.load_undefined(Some(expression.span))?;
                if undef != target {
                    self.builder.emit_mov(target, undef);
                    self.temp_top = target;
                }
                Ok(target)
            }
            UnaryOperator::Delete => self.compile_delete_expression(expression),
        }
    }

    pub(crate) fn compile_delete_expression(
        &mut self,
        expression: &UnaryExpression,
    ) -> Result<u8, CodegenError> {
        match &expression.argument {
            Expression::Identifier(identifier) => {
                let reg = self.alloc_temp(Some(expression.span))?;
                if self.name_slots.contains_key(&identifier.name) {
                    self.builder.emit_load_false(reg);
                } else {
                    self.builder.emit_load_true(reg);
                }
                Ok(reg)
            }
            Expression::Member(member) => match &member.property {
                MemberProperty::Identifier(identifier) => {
                    let object = self.compile_expression(&member.object)?;
                    let reg = self.alloc_temp(Some(expression.span))?;
                    let slot = self.property_slot(&identifier.name)?;
                    self.builder.emit_delete_prop(reg, object, slot);
                    Ok(reg)
                }
                MemberProperty::Computed {
                    expression: prop_expr,
                    ..
                } => {
                    let object = self.compile_expression(&member.object)?;
                    let key = self.compile_expression(prop_expr)?;
                    let reg = self.alloc_temp(Some(expression.span))?;
                    self.builder.emit_delete_prop(reg, object, key);
                    Ok(reg)
                }
                MemberProperty::PrivateName(identifier) => Err(CodegenError::Unsupported {
                    feature: "private members",
                    span: identifier.span,
                }),
            },
            _ => {
                let reg = self.compile_expression(&expression.argument)?;
                self.builder.emit_load_true(reg);
                Ok(reg)
            }
        }
    }

    pub(crate) fn compile_update_expression(
        &mut self,
        expression: &UpdateExpression,
    ) -> Result<u8, CodegenError> {
        match &expression.argument {
            Expression::Identifier(identifier) => {
                let current = self.compile_identifier_current(identifier)?;
                match expression.operator {
                    UpdateOperator::Increment => self.builder.emit_inc(current),
                    UpdateOperator::Decrement => self.builder.emit_dec(current),
                }

                if expression.prefix {
                    self.builder.emit_mov(current, ACC);
                    self.write_name_binding(&identifier.name, current)?;
                    Ok(current)
                } else {
                    let updated = self.alloc_temp(Some(expression.span))?;
                    self.builder.emit_mov(updated, ACC);
                    self.write_name_binding(&identifier.name, updated)?;
                    self.temp_top = current;
                    Ok(current)
                }
            }
            Expression::Member(member) => {
                let (object_reg, key_reg, immediate_key) = self.compile_member_target(member)?;
                let current = if let Some(key) = immediate_key {
                    self.builder.emit_get_prop(object_reg, object_reg, key);
                    object_reg
                } else {
                    let key_reg = key_reg.expect("computed member key");
                    self.builder.emit_get_prop_acc(object_reg, key_reg);
                    self.builder.emit_mov(object_reg, ACC);
                    object_reg
                };

                match expression.operator {
                    UpdateOperator::Increment => self.builder.emit_inc(current),
                    UpdateOperator::Decrement => self.builder.emit_dec(current),
                }

                if expression.prefix {
                    self.builder.emit_mov(current, ACC);
                    if let Some(key) = immediate_key {
                        self.builder.emit_set_prop(current, object_reg, key);
                    } else {
                        let key_reg = key_reg.expect("computed member key");
                        self.builder.emit_mov(ACC, current);
                        self.builder.emit_set_prop_acc(object_reg, key_reg);
                    }
                    Ok(current)
                } else {
                    let updated = self.alloc_temp(Some(expression.span))?;
                    self.builder.emit_mov(updated, ACC);
                    if let Some(key) = immediate_key {
                        self.builder.emit_set_prop(updated, object_reg, key);
                    } else {
                        let key_reg = key_reg.expect("computed member key");
                        self.builder.emit_mov(ACC, updated);
                        self.builder.emit_set_prop_acc(object_reg, key_reg);
                    }
                    self.temp_top = current;
                    Ok(current)
                }
            }
            other => Err(CodegenError::Unsupported {
                feature: "update target",
                span: other.span(),
            }),
        }
    }

    pub(crate) fn compile_binary_expression(
        &mut self,
        expression: &BinaryExpression,
    ) -> Result<u8, CodegenError> {
        let left = self.compile_expression(&expression.left)?;
        let right = self.compile_expression(&expression.right)?;

        match expression.operator {
            BinaryOperator::Add => {
                self.builder.emit_add(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Subtract => {
                self.builder.emit_mov(ACC, left);
                self.builder.emit_sub_acc(right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Multiply => {
                self.builder.emit_mov(ACC, left);
                self.builder.emit_mul_acc(right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Divide => {
                self.builder.emit_mov(ACC, left);
                self.builder.emit_div_acc(right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Modulo => {
                self.builder.emit_mod(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Exponentiate => {
                self.builder.emit_pow(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::LeftShift => {
                self.builder.emit_shl(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::SignedRightShift => {
                self.builder.emit_shr(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::UnsignedRightShift => {
                self.builder.emit_ushr(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::LessThan => {
                self.builder.emit_lt(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::LessThanOrEqual => {
                self.builder.emit_lte(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::GreaterThan => {
                self.builder.emit_lt(right, left);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::GreaterThanOrEqual => {
                self.builder.emit_lte(right, left);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Equality => {
                self.builder.emit_eq(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::StrictEquality => {
                self.builder.emit_strict_eq(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Inequality => {
                self.builder.emit_eq(left, right);
                self.builder.emit_mov(left, ACC);
                let false_branch = self.emit_placeholder_jmp_false(left);
                self.builder.emit_load_false(left);
                let end_jump = self.emit_placeholder_jmp();
                let true_start = self.builder.len();
                self.patch_jump(
                    false_branch,
                    true_start,
                    JumpPatchKind::JmpFalse { reg: left },
                );
                self.builder.emit_load_true(left);
                let end = self.builder.len();
                self.patch_jump(end_jump, end, JumpPatchKind::Jmp);
            }
            BinaryOperator::StrictInequality => {
                self.builder.emit_strict_neq(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::BitwiseAnd => {
                self.builder.emit_bit_and(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::BitwiseOr => {
                self.builder.emit_bit_or(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::BitwiseXor => {
                self.builder.emit_bit_xor(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::In => {
                self.builder.emit_in(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::PrivateIn => {
                self.builder.emit_private_in(left, right);
                self.builder.emit_mov(left, ACC);
            }
            BinaryOperator::Instanceof => {
                self.builder.emit_instanceof(left, right);
                self.builder.emit_mov(left, ACC);
            }
        }

        self.temp_top = left;
        Ok(left)
    }

    pub(crate) fn compile_logical_expression(
        &mut self,
        expression: &LogicalExpression,
    ) -> Result<u8, CodegenError> {
        let left = self.compile_expression(&expression.left)?;
        let right = self.compile_expression(&expression.right)?;

        match expression.operator {
            LogicalOperator::And => self.builder.emit_logical_and(left, right),
            LogicalOperator::Or => self.builder.emit_logical_or(left, right),
            LogicalOperator::NullishCoalescing => self.builder.emit_nullish_coalesce(left, right),
        }
        self.builder.emit_mov(left, ACC);
        self.temp_top = left;
        Ok(left)
    }

    pub(crate) fn compile_assignment_expression(
        &mut self,
        expression: &AssignmentExpression,
    ) -> Result<u8, CodegenError> {
        match &expression.left {
            Expression::Identifier(identifier) => {
                let value = match expression.operator {
                    AssignmentOperator::Assign => self.compile_expression(&expression.right)?,
                    AssignmentOperator::AddAssign => {
                        let left = self.compile_identifier_current(identifier)?;
                        let right = self.compile_expression(&expression.right)?;
                        self.builder.emit_add(left, right);
                        self.builder.emit_mov(left, ACC);
                        self.temp_top = left;
                        left
                    }
                    AssignmentOperator::SubAssign => {
                        let left = self.compile_identifier_current(identifier)?;
                        let right = self.compile_expression(&expression.right)?;
                        self.builder.emit_mov(ACC, left);
                        self.builder.emit_sub_acc(right);
                        self.builder.emit_mov(left, ACC);
                        self.temp_top = left;
                        left
                    }
                    AssignmentOperator::MulAssign => {
                        let left = self.compile_identifier_current(identifier)?;
                        let right = self.compile_expression(&expression.right)?;
                        self.builder.emit_mov(ACC, left);
                        self.builder.emit_mul_acc(right);
                        self.builder.emit_mov(left, ACC);
                        self.temp_top = left;
                        left
                    }
                    AssignmentOperator::DivAssign => {
                        let left = self.compile_identifier_current(identifier)?;
                        let right = self.compile_expression(&expression.right)?;
                        self.builder.emit_mov(ACC, left);
                        self.builder.emit_div_acc(right);
                        self.builder.emit_mov(left, ACC);
                        self.temp_top = left;
                        left
                    }
                    _ => {
                        return Err(CodegenError::Unsupported {
                            feature: "assignment operator",
                            span: expression.span,
                        });
                    }
                };

                self.write_name_binding(&identifier.name, value)?;
                self.temp_top = value;
                Ok(value)
            }
            Expression::Member(member) => {
                let (object_reg, key_reg, immediate_key) = self.compile_member_target(member)?;
                let value = match expression.operator {
                    AssignmentOperator::Assign => self.compile_expression(&expression.right)?,
                    AssignmentOperator::AddAssign
                    | AssignmentOperator::SubAssign
                    | AssignmentOperator::MulAssign
                    | AssignmentOperator::DivAssign => {
                        let current = if let Some(key) = immediate_key {
                            self.builder.emit_get_prop(object_reg, object_reg, key);
                            object_reg
                        } else {
                            let key_reg = key_reg.expect("computed member key");
                            self.builder.emit_get_prop_acc(object_reg, key_reg);
                            self.builder.emit_mov(object_reg, ACC);
                            object_reg
                        };
                        let rhs = self.compile_expression(&expression.right)?;
                        match expression.operator {
                            AssignmentOperator::AddAssign => self.builder.emit_add(current, rhs),
                            AssignmentOperator::SubAssign => {
                                self.builder.emit_mov(ACC, current);
                                self.builder.emit_sub_acc(rhs);
                            }
                            AssignmentOperator::MulAssign => {
                                self.builder.emit_mov(ACC, current);
                                self.builder.emit_mul_acc(rhs);
                            }
                            AssignmentOperator::DivAssign => {
                                self.builder.emit_mov(ACC, current);
                                self.builder.emit_div_acc(rhs);
                            }
                            _ => unreachable!(),
                        }
                        self.builder.emit_mov(current, ACC);
                        self.temp_top = current;
                        current
                    }
                    _ => {
                        return Err(CodegenError::Unsupported {
                            feature: "assignment operator",
                            span: expression.span,
                        });
                    }
                };

                if let Some(key) = immediate_key {
                    self.builder.emit_set_prop(value, object_reg, key);
                } else {
                    let key_reg = key_reg.expect("computed member key");
                    self.builder.emit_mov(ACC, value);
                    self.builder.emit_set_prop_acc(object_reg, key_reg);
                }
                self.temp_top = value;
                Ok(value)
            }
            other => Err(CodegenError::Unsupported {
                feature: "assignment target",
                span: other.span(),
            }),
        }
    }

    pub(crate) fn compile_conditional_expression(
        &mut self,
        expression: &ConditionalExpression,
    ) -> Result<u8, CodegenError> {
        let test = self.compile_expression(&expression.test)?;
        let target = test;
        let false_jump = self.emit_placeholder_jmp_false(test);

        let consequent = self.compile_expression(&expression.consequent)?;
        if consequent != target {
            self.builder.emit_mov(target, consequent);
        }
        let end_jump = self.emit_placeholder_jmp();

        let alternate_start = self.builder.len();
        self.patch_jump(
            false_jump,
            alternate_start,
            JumpPatchKind::JmpFalse { reg: test },
        );

        let alternate = self.compile_expression(&expression.alternate)?;
        if alternate != target {
            self.builder.emit_mov(target, alternate);
        }

        let end = self.builder.len();
        self.patch_jump(end_jump, end, JumpPatchKind::Jmp);
        self.temp_top = target;
        Ok(target)
    }

    pub(crate) fn compile_sequence_expression(
        &mut self,
        expression: &SequenceExpression,
    ) -> Result<u8, CodegenError> {
        let mut last = self.load_undefined(Some(expression.span))?;
        for expr in &expression.expressions {
            last = self.compile_expression(expr)?;
        }
        Ok(last)
    }

    pub(crate) fn compile_fixed_call_arguments(
        &mut self,
        callee: u8,
        arguments: &[CallArgument],
        span: Span,
    ) -> Result<u8, CodegenError> {
        for (index, argument) in arguments.iter().enumerate() {
            match argument {
                CallArgument::Expression(expression) => {
                    let arg_reg = self.compile_expression(expression)?;
                    let expected = callee as usize + 1 + index;
                    if expected >= ACC as usize {
                        return Err(CodegenError::RegisterOverflow { span: Some(span) });
                    }
                    if arg_reg != expected as u8 {
                        self.builder.emit_mov(expected as u8, arg_reg);
                        self.temp_top = self.temp_top.max(expected as u8);
                    }
                }
                CallArgument::Spread { span, .. } => {
                    return Err(CodegenError::Unsupported {
                        feature: "mixed spread calls",
                        span: *span,
                    });
                }
            }
        }

        Ok(arguments.len().min(u8::MAX as usize) as u8)
    }

    pub(crate) fn compile_call_expression(
        &mut self,
        expression: &CallExpression,
    ) -> Result<u8, CodegenError> {
        if expression.optional {
            return Err(CodegenError::Unsupported {
                feature: "optional call",
                span: expression.span,
            });
        }

        if let Expression::Member(member) = &expression.callee {
            if expression.arguments.is_empty() {
                let object = self.compile_expression(&member.object)?;
                match &member.property {
                    MemberProperty::Identifier(identifier) => {
                        let slot = self.property_slot(&identifier.name)?;
                        self.builder.emit_call_method_ic(object, slot);
                        self.builder.emit_mov(object, ACC);
                        self.temp_top = object;
                        return Ok(object);
                    }
                    MemberProperty::Computed { expression, .. } => {
                        let key = self.compile_expression(expression)?;
                        self.builder.emit_get_prop_acc_call(object, key);
                        self.builder.emit_mov(object, ACC);
                        self.temp_top = object;
                        return Ok(object);
                    }
                    MemberProperty::PrivateName(identifier) => {
                        return Err(CodegenError::Unsupported {
                            feature: "private method calls",
                            span: identifier.span,
                        });
                    }
                }
            }

            if expression.arguments.len() <= 2
                && expression
                    .arguments
                    .iter()
                    .all(|argument| matches!(argument, CallArgument::Expression(_)))
                && let MemberProperty::Identifier(identifier) = &member.property
            {
                let object = self.compile_expression(&member.object)?;
                let arg_count = self.compile_fixed_call_arguments(
                    object,
                    &expression.arguments,
                    expression.span,
                )?;
                let slot = self.property_slot(&identifier.name)?;
                match arg_count {
                    1 => self.builder.emit_call_method1(object, u16::from(slot)),
                    2 => self.builder.emit_call_method2(object, u16::from(slot)),
                    _ => unreachable!("member fast path only handles one or two arguments"),
                }
                self.builder.emit_mov(object, ACC);
                self.temp_top = object;
                return Ok(object);
            }

            let (callee, this_reg) = self.compile_member_callee(member)?;

            if let [CallArgument::Spread { argument, .. }] = expression.arguments.as_slice() {
                let array_reg = self.compile_expression(argument)?;
                self.builder.emit_call_this_var(callee, this_reg, array_reg);
                self.builder.emit_mov(callee, ACC);
                self.temp_top = callee;
                return Ok(callee);
            }

            let arg_count =
                self.compile_fixed_call_arguments(callee, &expression.arguments, expression.span)?;
            self.builder.emit_call_this(callee, this_reg, arg_count);
            self.builder.emit_mov(callee, ACC);
            self.temp_top = callee;
            return Ok(callee);
        }

        if !matches!(expression.callee, Expression::Member(_))
            && let Some((source, imm)) = extract_call1_sub_i_arg(&expression.arguments)?
        {
            let callee = self.compile_expression(&expression.callee)?;
            let source = self.compile_readonly_expression(source)?;
            self.builder.emit_call1_sub_i(callee, source, imm);
            let result = if self.current_self_upvalue_reg == Some(callee) {
                self.alloc_temp(Some(expression.span))?
            } else {
                callee
            };
            self.builder.emit_mov(result, ACC);
            self.temp_top = result;
            return Ok(result);
        }

        let callee = self.compile_expression(&expression.callee)?;

        if let [CallArgument::Spread { argument, .. }] = expression.arguments.as_slice() {
            let array_reg = self.compile_expression(argument)?;
            if array_reg != callee + 1 {
                return Err(CodegenError::Unsupported {
                    feature: "spread call with non-contiguous arguments",
                    span: expression.span,
                });
            }
            self.builder.emit_call_var(callee, array_reg);
            let result = if self.current_self_upvalue_reg == Some(callee) {
                self.alloc_temp(Some(expression.span))?
            } else {
                callee
            };
            self.builder.emit_mov(result, ACC);
            self.temp_top = result;
            return Ok(result);
        }

        let arg_count =
            self.compile_fixed_call_arguments(callee, &expression.arguments, expression.span)?;
        self.builder.emit_call(callee, arg_count);
        let result = if self.current_self_upvalue_reg == Some(callee) {
            self.alloc_temp(Some(expression.span))?
        } else {
            callee
        };
        self.builder.emit_mov(result, ACC);
        self.temp_top = result;
        Ok(result)
    }

    pub(crate) fn compile_member_expression(
        &mut self,
        expression: &MemberExpression,
    ) -> Result<u8, CodegenError> {
        if expression.optional {
            return Err(CodegenError::Unsupported {
                feature: "optional chaining",
                span: expression.span,
            });
        }

        let object = self.compile_expression(&expression.object)?;
        match &expression.property {
            MemberProperty::Identifier(identifier) => {
                let slot = self.property_slot(&identifier.name)?;
                self.builder.emit_get_prop(object, object, slot);
                Ok(object)
            }
            MemberProperty::Computed { expression, .. } => {
                let key = self.compile_expression(expression)?;
                self.builder.emit_get_prop_acc(object, key);
                self.builder.emit_mov(object, ACC);
                self.temp_top = object;
                Ok(object)
            }
            MemberProperty::PrivateName(identifier) => Err(CodegenError::Unsupported {
                feature: "private members",
                span: identifier.span,
            }),
        }
    }

    pub(crate) fn compile_new_expression(
        &mut self,
        expression: &NewExpression,
    ) -> Result<u8, CodegenError> {
        let callee = self.compile_expression(&expression.callee)?;

        if let [CallArgument::Spread { span, .. }] = expression.arguments.as_slice() {
            return Err(CodegenError::Unsupported {
                feature: "spread in `new` expressions",
                span: *span,
            });
        }

        for argument in &expression.arguments {
            match argument {
                CallArgument::Expression(expression) => {
                    let _ = self.compile_expression(expression)?;
                }
                CallArgument::Spread { span, .. } => {
                    return Err(CodegenError::Unsupported {
                        feature: "spread in `new` expressions",
                        span: *span,
                    });
                }
            }
        }

        self.builder.emit_construct(
            callee,
            expression.arguments.len().min(u8::MAX as usize) as u8,
        );
        self.builder.emit_mov(callee, ACC);
        self.temp_top = callee;
        Ok(callee)
    }

    pub(crate) fn compile_member_target(
        &mut self,
        member: &MemberExpression,
    ) -> Result<(u8, Option<u8>, Option<u8>), CodegenError> {
        if member.optional {
            return Err(CodegenError::Unsupported {
                feature: "optional chaining",
                span: member.span,
            });
        }

        let object_reg = self.compile_expression(&member.object)?;
        match &member.property {
            MemberProperty::Identifier(identifier) => {
                let slot = self.property_slot(&identifier.name)?;
                Ok((object_reg, None, Some(slot)))
            }
            MemberProperty::Computed { expression, .. } => {
                let key_reg = self.compile_expression(expression)?;
                Ok((object_reg, Some(key_reg), None))
            }
            MemberProperty::PrivateName(identifier) => Err(CodegenError::Unsupported {
                feature: "private members",
                span: identifier.span,
            }),
        }
    }

    pub(crate) fn emit_store_property(
        &mut self,
        object_reg: u8,
        key: &PropertyKey,
        value_reg: u8,
    ) -> Result<(), CodegenError> {
        match key {
            PropertyKey::Identifier(identifier) => {
                let slot = self.property_slot(&identifier.name)?;
                self.builder.emit_set_prop(value_reg, object_reg, slot);
            }
            PropertyKey::String(StringLiteral { value, .. }) => {
                let slot = self.property_slot(value)?;
                self.builder.emit_set_prop(value_reg, object_reg, slot);
            }
            PropertyKey::Number(number) => {
                let key_reg = self.compile_numeric_key(number)?;
                self.builder.emit_mov(ACC, value_reg);
                self.builder.emit_set_prop_acc(object_reg, key_reg);
            }
            PropertyKey::Computed { expression, .. } => {
                let key_reg = self.compile_expression(expression)?;
                self.builder.emit_mov(ACC, value_reg);
                self.builder.emit_set_prop_acc(object_reg, key_reg);
            }
            PropertyKey::PrivateName(identifier) => {
                let slot = self.private_property_slot(&identifier.name)?;
                self.builder.emit_set_private_prop(value_reg, object_reg, slot);
            }
        }
        Ok(())
    }

    pub(crate) fn compile_numeric_key(
        &mut self,
        literal: &NumberLiteral,
    ) -> Result<u8, CodegenError> {
        let reg = self.alloc_temp(Some(literal.span))?;
        let value = parse_number_literal(literal)?;
        if value.fract() == 0.0 && value >= i16::MIN as f64 && value <= i16::MAX as f64 {
            self.builder.emit_load_i(reg, value as i16);
        } else {
            let index = self.builder.add_constant(make_number(value));
            self.builder.emit_load_k(reg, index);
        }
        Ok(reg)
    }

    pub(crate) fn compile_condition_jump_false(
        &mut self,
        expression: &Expression,
    ) -> Result<(usize, JumpPatchKind, u8), CodegenError> {
        if let Expression::Binary(binary) = expression
            && binary.operator == BinaryOperator::LessThanOrEqual
        {
            let lhs = self.compile_readonly_expression(&binary.left)?;
            let rhs = self.compile_readonly_expression(&binary.right)?;
            let jump = self.emit_placeholder_jmp_lte_false(lhs, rhs);
            return Ok((jump, JumpPatchKind::JmpLteFalse { lhs, rhs }, lhs.max(rhs)));
        }

        let reg = self.compile_expression(expression)?;
        let jump = self.emit_placeholder_jmp_false(reg);
        Ok((jump, JumpPatchKind::JmpFalse { reg }, reg))
    }
}
