use super::*;

impl Codegen {
    pub(crate) fn compile_statement_list(
        &mut self,
        statements: &[Statement],
        root_script: bool,
    ) -> Result<Option<u8>, CodegenError> {
        let mut last_value = None;
        for statement in statements {
            let value = self.compile_statement(statement, root_script)?;
            if value.is_some() {
                last_value = value;
            }
        }
        Ok(last_value)
    }

    pub(crate) fn compile_statement(
        &mut self,
        statement: &Statement,
        root_script: bool,
    ) -> Result<Option<u8>, CodegenError> {
        match statement {
            Statement::Directive(_) | Statement::Empty(_) | Statement::Debugger(_) => Ok(None),
            Statement::Block(block) => self.compile_block(block, !root_script),
            Statement::VariableDeclaration(declaration) => {
                self.compile_variable_declaration(declaration)?;
                Ok(None)
            }
            Statement::FunctionDeclaration(function) => {
                self.compile_function_declaration(function)?;
                Ok(None)
            }
            Statement::If(statement) => {
                self.compile_if_statement(statement, root_script)?;
                Ok(None)
            }
            Statement::While(statement) => {
                self.compile_while_statement(statement, root_script)?;
                Ok(None)
            }
            Statement::DoWhile(statement) => {
                self.compile_do_while_statement(statement, root_script)?;
                Ok(None)
            }
            Statement::For(statement) => {
                self.compile_for_statement(statement, root_script)?;
                Ok(None)
            }
            Statement::Return(statement) => {
                self.compile_return_statement(statement)?;
                Ok(None)
            }
            Statement::Break(jump) => {
                if let Some(label) = &jump.label {
                    let Some((label_index, break_sink)) = self
                        .label_stack
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, ctx)| ctx.name == label.name)
                        .map(|(index, ctx)| (index, ctx.break_sink))
                    else {
                        return Err(CodegenError::InvalidBreak { span: jump.span });
                    };
                    if self.finally_stack.last().is_some_and(|ctx| {
                        !self.finally_escape_patch_stack.is_empty() && label_index < ctx.label_depth
                    }) {
                        self.queue_labeled_break_through_finally(label, jump.span)?;
                        return Ok(None);
                    }
                    let patch = self.emit_placeholder_jmp();
                    self.queue_jump_sink_patch(break_sink, patch);
                    return Ok(None);
                }
                let Some((control_index, break_sink)) = self
                    .control_stack
                    .iter()
                    .enumerate()
                    .next_back()
                    .map(|(index, ctx)| (index, ctx.break_sink))
                else {
                    return Err(CodegenError::InvalidBreak { span: jump.span });
                };
                if self.finally_stack.last().is_some_and(|ctx| {
                    !self.finally_escape_patch_stack.is_empty() && control_index < ctx.control_depth
                }) {
                    self.queue_loop_transfer_through_finally(jump.span, false)?;
                    return Ok(None);
                }
                let patch = self.emit_placeholder_jmp();
                self.queue_jump_sink_patch(break_sink, patch);
                Ok(None)
            }
            Statement::Continue(jump) => {
                if jump.label.is_some() {
                    return Err(CodegenError::Unsupported {
                        feature: "labeled continue",
                        span: jump.span,
                    });
                }
                let Some((control_index, continue_sink)) = self
                    .control_stack
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(index, ctx)| ctx.continue_sink.map(|sink| (index, sink)))
                else {
                    return Err(CodegenError::InvalidContinue { span: jump.span });
                };
                if self.finally_stack.last().is_some_and(|ctx| {
                    !self.finally_escape_patch_stack.is_empty() && control_index < ctx.control_depth
                }) {
                    self.queue_loop_transfer_through_finally(jump.span, true)?;
                    return Ok(None);
                }
                let patch = self.emit_placeholder_jmp();
                self.queue_jump_sink_patch(continue_sink, patch);
                Ok(None)
            }
            Statement::Expression(ExpressionStatement { expression, .. }) => {
                self.compile_expression(expression).map(Some)
            }
            Statement::Labeled(node) => {
                let break_sink = self.alloc_jump_sink();
                self.label_stack.push(LabelContext {
                    name: node.label.name.clone(),
                    break_sink,
                });
                self.compile_statement(&node.body, root_script)?;
                let label_ctx = self.label_stack.pop().expect("label context");
                let end = self.builder.len();
                self.resolve_jump_sink(label_ctx.break_sink, end);
                Ok(None)
            }
            Statement::ImportDeclaration(node) => Err(CodegenError::Unsupported {
                feature: "imports",
                span: node.span,
            }),
            Statement::ExportDeclaration(node) => Err(CodegenError::Unsupported {
                feature: "exports",
                span: node.span(),
            }),
            Statement::ClassDeclaration(node) => {
                self.compile_class_declaration(node)?;
                Ok(None)
            }
            Statement::Switch(node) => {
                self.compile_switch_statement(node, root_script)?;
                Ok(None)
            }
            Statement::Throw(node) => {
                self.compile_throw_statement(node)?;
                Ok(None)
            }
            Statement::Try(node) => {
                self.compile_try_statement(node)?;
                Ok(None)
            }
            Statement::With(node) => Err(CodegenError::Unsupported {
                feature: "with",
                span: node.span,
            }),
        }
    }

    pub(crate) fn compile_block(
        &mut self,
        block: &BlockStatement,
        create_scope: bool,
    ) -> Result<Option<u8>, CodegenError> {
        if !create_scope {
            return self.compile_statement_list(&block.body, false);
        }

        let saved_top = self.temp_top;
        self.nested_scope_depth += 1;

        // Check if this block actually needs runtime bindings
        let needs_runtime_bindings = self.block_needs_runtime_bindings(block);
        self.enter_fast_name_scope_with_runtime_bindings(needs_runtime_bindings);

        if needs_runtime_bindings {
            self.builder.emit_enter(256);
            let _env_reg = self.alloc_temp(Some(block.span))?;
            self.builder.emit_create_env(_env_reg);
        }

        let last = self.compile_statement_list(&block.body, false)?;

        if needs_runtime_bindings {
            self.builder.emit_leave();
        }

        self.leave_fast_name_scope();
        self.nested_scope_depth = self.nested_scope_depth.saturating_sub(1);
        self.temp_top = saved_top;
        Ok(last)
    }

    pub(crate) fn compile_if_statement(
        &mut self,
        statement: &IfStatement,
        root_script: bool,
    ) -> Result<(), CodegenError> {
        if self.compile_fused_if_return(statement)? {
            return Ok(());
        }

        let (false_jump, false_kind, test_top) =
            self.compile_condition_jump_false(&statement.test)?;
        self.compile_statement(&statement.consequent, root_script)?;

        if let Some(alternate) = &statement.alternate {
            let end_jump = self.emit_placeholder_jmp();
            let alternate_start = self.builder.len();
            self.patch_jump(false_jump, alternate_start, false_kind);
            self.compile_statement(alternate, root_script)?;
            let end = self.builder.len();
            self.patch_jump(end_jump, end, JumpPatchKind::Jmp);
        } else {
            let end = self.builder.len();
            self.patch_jump(false_jump, end, false_kind);
        }

        self.temp_top = test_top.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_fused_if_return(
        &mut self,
        statement: &IfStatement,
    ) -> Result<bool, CodegenError> {
        if statement.alternate.is_some() {
            return Ok(false);
        }

        let Statement::Return(return_stmt) = statement.consequent.as_ref() else {
            return Ok(false);
        };
        let Some(Expression::Identifier(return_id)) = return_stmt.argument.as_ref() else {
            return Ok(false);
        };
        let Expression::Binary(binary) = &statement.test else {
            return Ok(false);
        };
        if binary.operator != BinaryOperator::LessThanOrEqual {
            return Ok(false);
        }
        let Expression::Identifier(lhs_id) = &binary.left else {
            return Ok(false);
        };
        if lhs_id.name != return_id.name {
            return Ok(false);
        }

        let lhs = self.compile_identifier_current(lhs_id)?;
        let rhs = self.compile_readonly_expression(&binary.right)?;
        self.builder.emit_ret_if_lte_i(lhs, rhs, lhs);
        self.temp_top = lhs.max(rhs).saturating_sub(1);
        Ok(true)
    }

    pub(crate) fn compile_while_statement(
        &mut self,
        statement: &WhileStatement,
        root_script: bool,
    ) -> Result<(), CodegenError> {
        let loop_start = self.builder.len();
        let (exit_jump, exit_kind, test_top) =
            self.compile_condition_jump_false(&statement.test)?;

        let break_sink = self.alloc_jump_sink();
        let continue_sink = self.alloc_jump_sink();
        self.control_stack
            .push(ControlContext::loop_context(break_sink, continue_sink));
        self.compile_statement(&statement.body, root_script)?;
        let loop_ctx = self.control_stack.pop().expect("loop context");

        let continue_target = loop_start;
        self.resolve_jump_sink(
            loop_ctx.continue_sink.expect("loop continue sink"),
            continue_target,
        );

        self.builder
            .emit_jmp(offset_to(loop_start, self.builder.len())?);

        let end = self.builder.len();
        self.patch_jump(exit_jump, end, exit_kind);
        self.resolve_jump_sink(loop_ctx.break_sink, end);
        self.temp_top = test_top.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_do_while_statement(
        &mut self,
        statement: &DoWhileStatement,
        root_script: bool,
    ) -> Result<(), CodegenError> {
        let body_start = self.builder.len();
        let break_sink = self.alloc_jump_sink();
        let continue_sink = self.alloc_jump_sink();
        self.control_stack
            .push(ControlContext::loop_context(break_sink, continue_sink));
        self.compile_statement(&statement.body, root_script)?;
        let test_start = self.builder.len();
        let test_reg = self.compile_expression(&statement.test)?;
        self.builder
            .emit_jmp_true(test_reg, offset_to(body_start, self.builder.len())?);

        let loop_ctx = self.control_stack.pop().expect("loop context");
        self.resolve_jump_sink(
            loop_ctx.continue_sink.expect("loop continue sink"),
            test_start,
        );
        let end = self.builder.len();
        self.resolve_jump_sink(loop_ctx.break_sink, end);
        self.temp_top = test_reg.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_for_statement(
        &mut self,
        statement: &ForStatement,
        root_script: bool,
    ) -> Result<(), CodegenError> {
        match statement {
            ForStatement::Classic(classic) => {
                self.compile_for_classic_statement(classic, root_script)
            }
            ForStatement::In(node) => Err(CodegenError::Unsupported {
                feature: "for-in",
                span: node.span,
            }),
            ForStatement::Of(node) => self.compile_for_of_statement(node, root_script),
        }
    }

    pub(crate) fn compile_for_classic_statement(
        &mut self,
        statement: &ForClassicStatement,
        root_script: bool,
    ) -> Result<(), CodegenError> {
        self.nested_scope_depth += 1;

        // Check if this loop actually needs runtime bindings
        let needs_runtime_bindings =
            self.for_statement_needs_runtime_bindings(&ForStatement::Classic(statement.clone()));
        self.enter_fast_name_scope_with_runtime_bindings(needs_runtime_bindings);

        if needs_runtime_bindings {
            self.builder.emit_enter(256);
            let _env_reg = self.alloc_temp(Some(statement.span))?;
            self.builder.emit_create_env(_env_reg);
        }

        if let Some(init) = &statement.init {
            match init {
                ForInit::VariableDeclaration(declaration) => {
                    self.compile_variable_declaration(declaration)?;
                }
                ForInit::Expression(expression) => {
                    let reg = self.compile_expression(expression)?;
                    self.temp_top = reg.saturating_sub(1);
                }
            }
        }

        let loop_start = self.builder.len();
        let exit_jump = if let Some(test) = &statement.test {
            let (patch, kind, top) = self.compile_condition_jump_false(test)?;
            Some((patch, kind, top))
        } else {
            None
        };

        let break_sink = self.alloc_jump_sink();
        let continue_sink = self.alloc_jump_sink();
        self.control_stack
            .push(ControlContext::loop_context(break_sink, continue_sink));
        self.compile_statement(&statement.body, root_script)?;
        let loop_ctx = self.control_stack.pop().expect("loop context");

        let continue_target = self.builder.len();
        self.resolve_jump_sink(
            loop_ctx.continue_sink.expect("loop continue sink"),
            continue_target,
        );

        if let Some(update) = &statement.update {
            let reg = self.compile_expression(update)?;
            self.temp_top = reg.saturating_sub(1);
        }

        self.builder
            .emit_jmp(offset_to(loop_start, self.builder.len())?);

        let end = self.builder.len();
        if let Some((patch_pos, kind, test_top)) = exit_jump {
            self.patch_jump(patch_pos, end, kind);
            self.temp_top = test_top.saturating_sub(1);
        }
        self.resolve_jump_sink(loop_ctx.break_sink, end);

        if needs_runtime_bindings {
            self.builder.emit_leave();
        }

        self.leave_fast_name_scope();
        self.nested_scope_depth = self.nested_scope_depth.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_for_of_statement(
        &mut self,
        statement: &ast::ForEachStatement,
        root_script: bool,
    ) -> Result<(), CodegenError> {
        if statement.is_await {
            return Err(CodegenError::Unsupported {
                feature: "for-await-of",
                span: statement.span,
            });
        }

        self.nested_scope_depth += 1;

        // Check if this loop actually needs runtime bindings
        let needs_runtime_bindings =
            self.for_statement_needs_runtime_bindings(&ForStatement::Of(statement.clone()));
        self.enter_fast_name_scope_with_runtime_bindings(needs_runtime_bindings);

        if needs_runtime_bindings {
            self.builder.emit_enter(256);
            let _env_reg = self.alloc_temp(Some(statement.span))?;
            self.builder.emit_create_env(_env_reg);
        }

        let iterable_reg = self.compile_expression(&statement.right)?;
        let index_reg = self.alloc_temp(Some(statement.span))?;
        self.builder.emit_load_i(index_reg, 0);

        let loop_start = self.builder.len();
        let length_reg = self.alloc_temp(Some(statement.span))?;
        self.builder.emit_get_length_ic(length_reg, iterable_reg, 0);
        self.builder.emit_lt(index_reg, length_reg);
        let cond_reg = self.alloc_temp(Some(statement.span))?;
        self.builder.emit_mov(cond_reg, ACC);
        let exit_jump = self.emit_placeholder_jmp_false(cond_reg);

        self.builder.emit_get_prop_acc(iterable_reg, index_reg);
        let value_reg = self.alloc_temp(Some(statement.span))?;
        self.builder.emit_mov(value_reg, ACC);
        self.bind_for_each_left(&statement.left, value_reg, statement.span)?;

        let break_sink = self.alloc_jump_sink();
        let continue_sink = self.alloc_jump_sink();
        self.control_stack
            .push(ControlContext::loop_context(break_sink, continue_sink));
        self.compile_statement(&statement.body, root_script)?;
        let loop_ctx = self.control_stack.pop().expect("loop context");

        let continue_target = self.builder.len();
        self.resolve_jump_sink(
            loop_ctx.continue_sink.expect("loop continue sink"),
            continue_target,
        );

        self.builder.emit_inc(index_reg);
        self.builder.emit_mov(index_reg, ACC);
        self.builder
            .emit_jmp(offset_to(loop_start, self.builder.len())?);

        let end = self.builder.len();
        self.patch_jump(exit_jump, end, JumpPatchKind::JmpFalse { reg: cond_reg });
        self.resolve_jump_sink(loop_ctx.break_sink, end);

        if needs_runtime_bindings {
            self.builder.emit_leave();
        }

        self.leave_fast_name_scope();
        self.nested_scope_depth = self.nested_scope_depth.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_switch_statement(
        &mut self,
        statement: &ast::SwitchStatement,
        _root_script: bool,
    ) -> Result<(), CodegenError> {
        let discriminant_reg = self.compile_expression(&statement.discriminant)?;
        let mut pending_false_jump: Option<(usize, u8)> = None;
        let mut case_entry_jumps = Vec::new();
        let mut default_case = None;

        for (index, case) in statement.cases.iter().enumerate() {
            if let Some((patch, reg)) = pending_false_jump.take() {
                self.patch_jump(patch, self.builder.len(), JumpPatchKind::JmpFalse { reg });
            }

            if let Some(test) = &case.test {
                let case_reg = self.compile_expression(test)?;
                self.builder.emit_strict_eq(discriminant_reg, case_reg);
                let cond_reg = self.alloc_temp(Some(case.span))?;
                self.builder.emit_mov(cond_reg, ACC);
                let false_jump = self.emit_placeholder_jmp_false(cond_reg);
                let matched_jump = self.emit_placeholder_jmp();
                pending_false_jump = Some((false_jump, cond_reg));
                case_entry_jumps.push((index, matched_jump));
                self.temp_top = discriminant_reg;
            } else {
                default_case = Some(index);
            }
        }

        let default_dispatch = self.builder.len();
        let default_jump = self.emit_placeholder_jmp();
        if let Some((patch, reg)) = pending_false_jump.take() {
            self.patch_jump(patch, default_dispatch, JumpPatchKind::JmpFalse { reg });
        }

        let break_sink = self.alloc_jump_sink();
        self.control_stack
            .push(ControlContext::switch_context(break_sink));
        let mut case_starts = vec![None; statement.cases.len()];
        for (index, case) in statement.cases.iter().enumerate() {
            case_starts[index] = Some(self.builder.len());
            self.compile_statement_list(&case.consequent, false)?;
        }
        let switch_ctx = self.control_stack.pop().expect("switch context");
        let end = self.builder.len();
        self.resolve_jump_sink(switch_ctx.break_sink, end);

        for (index, jump_pos) in case_entry_jumps {
            let target = case_starts[index].unwrap_or(end);
            self.patch_jump(jump_pos, target, JumpPatchKind::Jmp);
        }
        let default_target = default_case
            .and_then(|index| case_starts[index])
            .unwrap_or(end);
        self.patch_jump(default_jump, default_target, JumpPatchKind::Jmp);

        self.temp_top = discriminant_reg.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_return_statement(
        &mut self,
        statement: &ReturnStatement,
    ) -> Result<(), CodegenError> {
        if let Some(argument) = &statement.argument {
            let reg = match argument {
                Expression::Identifier(identifier) => {
                    self.compile_identifier_current(identifier)?
                }
                _ => self.compile_expression(argument)?,
            };
            self.temp_top = reg.saturating_sub(1);
            if !self.finally_stack.is_empty() && !self.finally_escape_patch_stack.is_empty() {
                self.queue_completion_through_finally(COMPLETION_RETURN, Some(reg), None);
                return Ok(());
            }
            self.builder.emit_ret_reg(reg);
        } else {
            if !self.finally_stack.is_empty() && !self.finally_escape_patch_stack.is_empty() {
                let undefined_reg = self.load_undefined(Some(statement.span))?;
                self.temp_top = undefined_reg.saturating_sub(1);
                self.queue_completion_through_finally(COMPLETION_RETURN, Some(undefined_reg), None);
                return Ok(());
            }
            self.builder.emit_ret_u();
        }
        Ok(())
    }

    pub(crate) fn compile_throw_statement(
        &mut self,
        statement: &ast::ThrowStatement,
    ) -> Result<(), CodegenError> {
        let reg = self.compile_expression(&statement.argument)?;
        self.builder.emit_throw(reg);
        self.temp_top = reg.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_try_statement(
        &mut self,
        statement: &ast::TryStatement,
    ) -> Result<(), CodegenError> {
        if statement.finalizer.is_some() {
            return self.compile_try_finally_statement(statement);
        }

        let Some(handler) = &statement.handler else {
            return Err(CodegenError::Unsupported {
                feature: "try without catch",
                span: statement.span,
            });
        };

        let try_patch = self.builder.len();
        self.builder.emit_try(0);
        self.compile_statement_list(&statement.block.body, false)?;
        self.builder.emit_end_try();
        let skip_catch = self.emit_placeholder_jmp();

        let catch_start = self.builder.len();
        self.patch_jump(try_patch, catch_start, JumpPatchKind::Try);
        self.compile_catch_clause(handler)?;

        let end = self.builder.len();
        self.patch_jump(skip_catch, end, JumpPatchKind::Jmp);
        Ok(())
    }

    pub(crate) fn compile_try_finally_statement(
        &mut self,
        statement: &ast::TryStatement,
    ) -> Result<(), CodegenError> {
        let finalizer = statement.finalizer.as_ref().expect("try/finally statement");

        let mode_reg = self.alloc_temp(Some(statement.span))?;
        self.builder.emit_load_i(mode_reg, COMPLETION_NORMAL);
        let value_reg = self.load_undefined(Some(statement.span))?;
        let target_reg = self.alloc_temp(Some(statement.span))?;
        self.builder.emit_load_i(target_reg, 0);

        self.finally_stack.push(FinallyContext {
            mode_reg,
            value_reg,
            target_reg,
            control_depth: self.control_stack.len(),
            label_depth: self.label_stack.len(),
            deferred_jumps: Vec::new(),
        });

        let try_patch = self.builder.len();
        self.builder.emit_try(0);
        self.finally_escape_patch_stack.push(Vec::new());
        self.compile_statement_list(&statement.block.body, false)?;
        let try_escape_patches = self
            .finally_escape_patch_stack
            .pop()
            .expect("try escape patches");
        self.builder.emit_end_try();
        let skip_catch = self.emit_placeholder_jmp();

        let try_escape_start = self.builder.len();
        self.builder.emit_end_try();
        let try_escape_to_finalizer = self.emit_placeholder_jmp();
        for patch in try_escape_patches {
            self.patch_jump(patch, try_escape_start, JumpPatchKind::Jmp);
        }

        let catch_start = self.builder.len();
        self.patch_jump(try_patch, catch_start, JumpPatchKind::Try);

        let mut finalizer_entry_patches = vec![try_escape_to_finalizer];
        if let Some(handler) = &statement.handler {
            self.compile_catch_clause_with_finally(
                handler,
                mode_reg,
                value_reg,
                &mut finalizer_entry_patches,
            )?;
        } else {
            let exception_reg = self.alloc_temp(Some(statement.span))?;
            self.builder.emit_catch(exception_reg);
            self.builder.emit_finally();
            if exception_reg != value_reg {
                self.builder.emit_mov(value_reg, exception_reg);
            }
            self.builder.emit_load_i(mode_reg, COMPLETION_THROW);
        }

        let after_catch = self.builder.len();
        self.patch_jump(skip_catch, after_catch, JumpPatchKind::Jmp);

        let ctx = self.finally_stack.pop().expect("finally context");
        let finalizer_start = self.builder.len();
        for patch in finalizer_entry_patches {
            self.patch_jump(patch, finalizer_start, JumpPatchKind::Jmp);
        }

        let _ = self.compile_block(finalizer, true)?;
        self.emit_finally_completion_dispatch(ctx)?;
        Ok(())
    }

    pub(crate) fn compile_catch_clause(
        &mut self,
        clause: &ast::CatchClause,
    ) -> Result<(), CodegenError> {
        let saved_top = self.temp_top;
        self.nested_scope_depth += 1;

        // Check if this catch clause actually needs runtime bindings
        let needs_runtime_bindings =
            clause.param.is_some() || self.block_needs_runtime_bindings(&clause.body);
        self.enter_fast_name_scope_with_runtime_bindings(needs_runtime_bindings);

        if needs_runtime_bindings {
            self.builder.emit_enter(256);
            let _env_reg = self.alloc_temp(Some(clause.span))?;
            self.builder.emit_create_env(_env_reg);
        }

        let exception_reg = self.alloc_temp(Some(clause.span))?;
        self.builder.emit_catch(exception_reg);

        if let Some(param) = &clause.param {
            match param {
                Pattern::Identifier(identifier) => {
                    self.declare_name_binding(&identifier.name, exception_reg)?;
                }
                other => {
                    return Err(CodegenError::Unsupported {
                        feature: "complex catch parameter",
                        span: other.span(),
                    });
                }
            }
        }

        self.compile_statement_list(&clause.body.body, false)?;

        if needs_runtime_bindings {
            self.builder.emit_leave();
        }

        self.builder.emit_finally();
        self.leave_fast_name_scope();
        self.nested_scope_depth = self.nested_scope_depth.saturating_sub(1);
        self.temp_top = saved_top;
        Ok(())
    }

    pub(crate) fn compile_catch_clause_with_finally(
        &mut self,
        clause: &ast::CatchClause,
        mode_reg: u8,
        value_reg: u8,
        finalizer_entry_patches: &mut Vec<usize>,
    ) -> Result<(), CodegenError> {
        let saved_top = self.temp_top;
        self.nested_scope_depth += 1;

        let needs_runtime_bindings =
            clause.param.is_some() || self.block_needs_runtime_bindings(&clause.body);
        self.enter_fast_name_scope_with_runtime_bindings(needs_runtime_bindings);

        if needs_runtime_bindings {
            self.builder.emit_enter(256);
            let _env_reg = self.alloc_temp(Some(clause.span))?;
            self.builder.emit_create_env(_env_reg);
        }

        let exception_reg = self.alloc_temp(Some(clause.span))?;
        self.builder.emit_catch(exception_reg);
        self.builder.emit_finally();

        if let Some(param) = &clause.param {
            match param {
                Pattern::Identifier(identifier) => {
                    self.declare_name_binding(&identifier.name, exception_reg)?;
                }
                other => {
                    return Err(CodegenError::Unsupported {
                        feature: "complex catch parameter",
                        span: other.span(),
                    });
                }
            }
        }

        let catch_try_patch = self.builder.len();
        self.builder.emit_try(0);
        self.finally_escape_patch_stack.push(Vec::new());
        self.compile_statement_list(&clause.body.body, false)?;
        let catch_escape_patches = self
            .finally_escape_patch_stack
            .pop()
            .expect("catch escape patches");
        self.builder.emit_end_try();
        let skip_catch = self.emit_placeholder_jmp();

        let catch_escape_start = self.builder.len();
        self.builder.emit_end_try();
        let catch_escape_to_finalizer = self.emit_placeholder_jmp();
        for patch in catch_escape_patches {
            self.patch_jump(patch, catch_escape_start, JumpPatchKind::Jmp);
        }
        finalizer_entry_patches.push(catch_escape_to_finalizer);

        let synthetic_catch_start = self.builder.len();
        self.patch_jump(catch_try_patch, synthetic_catch_start, JumpPatchKind::Try);
        let thrown_reg = self.alloc_temp(Some(clause.span))?;
        self.builder.emit_catch(thrown_reg);
        self.builder.emit_finally();
        if thrown_reg != value_reg {
            self.builder.emit_mov(value_reg, thrown_reg);
        }
        self.builder.emit_load_i(mode_reg, COMPLETION_THROW);

        let after_catch = self.builder.len();
        self.patch_jump(skip_catch, after_catch, JumpPatchKind::Jmp);

        if needs_runtime_bindings {
            self.builder.emit_leave();
        }

        self.leave_fast_name_scope();
        self.nested_scope_depth = self.nested_scope_depth.saturating_sub(1);
        self.temp_top = saved_top;
        Ok(())
    }

    fn queue_completion_through_finally(
        &mut self,
        mode: i16,
        value_reg: Option<u8>,
        target_id: Option<i16>,
    ) {
        let Some(ctx) = self.finally_stack.last().cloned() else {
            return;
        };

        self.builder.emit_load_i(ctx.mode_reg, mode);
        if let Some(value_reg) = value_reg
            && value_reg != ctx.value_reg
        {
            self.builder.emit_mov(ctx.value_reg, value_reg);
        }
        if let Some(target_id) = target_id {
            self.builder.emit_load_i(ctx.target_reg, target_id);
        }

        let patch = self.emit_placeholder_jmp();
        self.finally_escape_patch_stack
            .last_mut()
            .expect("active finally escape route")
            .push(patch);
    }

    fn queue_loop_transfer_through_finally(
        &mut self,
        span: Span,
        is_continue: bool,
    ) -> Result<(), CodegenError> {
        let destination = if is_continue {
            self.control_stack
                .iter()
                .rev()
                .find_map(|ctx| {
                    ctx.continue_sink
                        .map(DeferredJumpDestination::ControlContinue)
                })
                .ok_or(CodegenError::InvalidContinue { span })?
        } else {
            self.control_stack
                .last()
                .map(|ctx| DeferredJumpDestination::ControlBreak(ctx.break_sink))
                .ok_or(CodegenError::InvalidBreak { span })?
        };

        let target = DeferredJumpTarget {
            id: self.next_deferred_jump_id,
            destination,
        };
        self.next_deferred_jump_id = self.next_deferred_jump_id.saturating_add(1);

        for ctx in &mut self.finally_stack {
            ctx.deferred_jumps.push(target);
        }

        self.queue_completion_through_finally(
            if is_continue {
                COMPLETION_CONTINUE
            } else {
                COMPLETION_BREAK
            },
            None,
            Some(target.id),
        );
        Ok(())
    }

    fn queue_labeled_break_through_finally(
        &mut self,
        label: &Identifier,
        span: Span,
    ) -> Result<(), CodegenError> {
        let break_sink = self
            .label_stack
            .iter()
            .rev()
            .find(|ctx| ctx.name == label.name)
            .map(|ctx| ctx.break_sink)
            .ok_or(CodegenError::InvalidBreak { span })?;

        let target = DeferredJumpTarget {
            id: self.next_deferred_jump_id,
            destination: DeferredJumpDestination::LabelBreak(break_sink),
        };
        self.next_deferred_jump_id = self.next_deferred_jump_id.saturating_add(1);

        for ctx in &mut self.finally_stack {
            ctx.deferred_jumps.push(target);
        }

        self.queue_completion_through_finally(COMPLETION_BREAK, None, Some(target.id));
        Ok(())
    }

    fn emit_finally_completion_dispatch(
        &mut self,
        ctx: FinallyContext,
    ) -> Result<(), CodegenError> {
        self.emit_completion_case(ctx.mode_reg, COMPLETION_RETURN, |this| {
            if let Some(parent) = this.finally_stack.last().cloned() {
                if ctx.value_reg != parent.value_reg {
                    this.builder.emit_mov(parent.value_reg, ctx.value_reg);
                }
                this.builder.emit_load_i(parent.mode_reg, COMPLETION_RETURN);
                let patch = this.emit_placeholder_jmp();
                this.finally_escape_patch_stack
                    .last_mut()
                    .expect("outer finally escape route")
                    .push(patch);
            } else {
                this.builder.emit_ret_reg(ctx.value_reg);
            }
            Ok(())
        })?;

        self.emit_completion_case(ctx.mode_reg, COMPLETION_THROW, |this| {
            if let Some(parent) = this.finally_stack.last().cloned() {
                if ctx.value_reg != parent.value_reg {
                    this.builder.emit_mov(parent.value_reg, ctx.value_reg);
                }
                this.builder.emit_load_i(parent.mode_reg, COMPLETION_THROW);
                let patch = this.emit_placeholder_jmp();
                this.finally_escape_patch_stack
                    .last_mut()
                    .expect("outer finally escape route")
                    .push(patch);
            } else {
                this.builder.emit_throw(ctx.value_reg);
            }
            Ok(())
        })?;

        self.emit_completion_case(ctx.mode_reg, COMPLETION_BREAK, |this| {
            if let Some(parent) = this.finally_stack.last().cloned() {
                if ctx.target_reg != parent.target_reg {
                    this.builder.emit_mov(parent.target_reg, ctx.target_reg);
                }
                this.builder.emit_load_i(parent.mode_reg, COMPLETION_BREAK);
                let patch = this.emit_placeholder_jmp();
                this.finally_escape_patch_stack
                    .last_mut()
                    .expect("outer finally escape route")
                    .push(patch);
            } else {
                this.emit_deferred_jump_dispatch(&ctx, false)?;
            }
            Ok(())
        })?;

        self.emit_completion_case(ctx.mode_reg, COMPLETION_CONTINUE, |this| {
            if let Some(parent) = this.finally_stack.last().cloned() {
                if ctx.target_reg != parent.target_reg {
                    this.builder.emit_mov(parent.target_reg, ctx.target_reg);
                }
                this.builder
                    .emit_load_i(parent.mode_reg, COMPLETION_CONTINUE);
                let patch = this.emit_placeholder_jmp();
                this.finally_escape_patch_stack
                    .last_mut()
                    .expect("outer finally escape route")
                    .push(patch);
            } else {
                this.emit_deferred_jump_dispatch(&ctx, true)?;
            }
            Ok(())
        })?;

        Ok(())
    }

    fn emit_completion_case(
        &mut self,
        mode_reg: u8,
        expected_mode: i16,
        action: impl FnOnce(&mut Self) -> Result<(), CodegenError>,
    ) -> Result<(), CodegenError> {
        let expected_reg = self.alloc_temp(None)?;
        self.builder.emit_load_i(expected_reg, expected_mode);
        self.builder.emit_strict_eq(mode_reg, expected_reg);
        let cond_reg = self.alloc_temp(None)?;
        self.builder.emit_mov(cond_reg, ACC);
        let skip_case = self.emit_placeholder_jmp_false(cond_reg);
        action(self)?;
        let end = self.builder.len();
        self.patch_jump(skip_case, end, JumpPatchKind::JmpFalse { reg: cond_reg });
        Ok(())
    }

    fn emit_deferred_jump_dispatch(
        &mut self,
        ctx: &FinallyContext,
        is_continue: bool,
    ) -> Result<(), CodegenError> {
        for target in ctx
            .deferred_jumps
            .iter()
            .copied()
            .filter(|target| match target.destination {
                DeferredJumpDestination::ControlContinue(_) => is_continue,
                DeferredJumpDestination::ControlBreak(_)
                | DeferredJumpDestination::LabelBreak(_) => !is_continue,
            })
        {
            let target_reg = self.alloc_temp(None)?;
            self.builder.emit_load_i(target_reg, target.id);
            self.builder.emit_strict_eq(ctx.target_reg, target_reg);
            let cond_reg = self.alloc_temp(None)?;
            self.builder.emit_mov(cond_reg, ACC);
            let skip_jump = self.emit_placeholder_jmp_false(cond_reg);
            let jump = self.emit_placeholder_jmp();
            match target.destination {
                DeferredJumpDestination::ControlBreak(break_sink) => {
                    self.queue_jump_sink_patch(break_sink, jump);
                }
                DeferredJumpDestination::ControlContinue(continue_sink) => {
                    self.queue_jump_sink_patch(continue_sink, jump);
                }
                DeferredJumpDestination::LabelBreak(break_sink) => {
                    self.queue_jump_sink_patch(break_sink, jump);
                }
            }
            let end = self.builder.len();
            self.patch_jump(skip_jump, end, JumpPatchKind::JmpFalse { reg: cond_reg });
        }
        Ok(())
    }

    pub(crate) fn bind_for_each_left(
        &mut self,
        left: &ast::ForLeft,
        value_reg: u8,
        span: Span,
    ) -> Result<(), CodegenError> {
        match left {
            ast::ForLeft::VariableDeclaration(declaration) => {
                if declaration.declarations.len() != 1 {
                    return Err(CodegenError::Unsupported {
                        feature: "multiple for-of bindings",
                        span: declaration.span,
                    });
                }
                let declarator = &declaration.declarations[0];
                if declarator.init.is_some() {
                    return Err(CodegenError::Unsupported {
                        feature: "initialized for-of bindings",
                        span: declarator.span,
                    });
                }
                self.bind_pattern_value(&declarator.pattern, value_reg, span, true)
            }
            ast::ForLeft::Pattern(pattern) => {
                self.bind_pattern_value(pattern, value_reg, span, false)
            }
            ast::ForLeft::Expression(expression) => {
                self.bind_assignment_target(expression, value_reg)
            }
        }
    }

    pub(crate) fn bind_pattern_value(
        &mut self,
        pattern: &Pattern,
        value_reg: u8,
        span: Span,
        declare: bool,
    ) -> Result<(), CodegenError> {
        match pattern {
            Pattern::Identifier(identifier) => {
                if declare {
                    self.declare_name_binding(&identifier.name, value_reg)?;
                } else {
                    self.write_name_binding(&identifier.name, value_reg)?;
                }
                Ok(())
            }
            Pattern::Assignment(pattern) => match pattern.left.as_ref() {
                Pattern::Identifier(identifier) => {
                    if declare {
                        self.declare_name_binding(&identifier.name, value_reg)?;
                    } else {
                        self.write_name_binding(&identifier.name, value_reg)?;
                    }
                    Ok(())
                }
                other => Err(CodegenError::Unsupported {
                    feature: "complex for-of binding",
                    span: other.span(),
                }),
            },
            _ => Err(CodegenError::Unsupported {
                feature: "complex for-of binding",
                span,
            }),
        }
    }

    pub(crate) fn bind_assignment_target(
        &mut self,
        expression: &Expression,
        value_reg: u8,
    ) -> Result<(), CodegenError> {
        match expression {
            Expression::Identifier(identifier) => {
                self.write_name_binding(&identifier.name, value_reg)?;
                Ok(())
            }
            Expression::Member(member) => {
                let (object_reg, key_reg, immediate_key) = self.compile_member_target(member)?;
                if let Some(key) = immediate_key {
                    self.builder.emit_set_prop(value_reg, object_reg, key);
                } else {
                    let key_reg = key_reg.expect("computed member key");
                    self.builder.emit_mov(ACC, value_reg);
                    self.builder.emit_set_prop_acc(object_reg, key_reg);
                }
                Ok(())
            }
            other => Err(CodegenError::Unsupported {
                feature: "for-of assignment target",
                span: other.span(),
            }),
        }
    }

    pub(crate) fn compile_variable_declaration(
        &mut self,
        declaration: &VariableDeclaration,
    ) -> Result<(), CodegenError> {
        for declarator in &declaration.declarations {
            self.compile_variable_declarator(declarator)?;
        }
        Ok(())
    }

    pub(crate) fn compile_variable_declarator(
        &mut self,
        declarator: &VariableDeclarator,
    ) -> Result<(), CodegenError> {
        match &declarator.pattern {
            Pattern::Identifier(identifier) => {
                let value_reg = if let Some(init) = &declarator.init {
                    self.compile_expression(init)?
                } else {
                    self.load_undefined(None)?
                };
                self.declare_name_binding(&identifier.name, value_reg)?;
                self.temp_top = value_reg.saturating_sub(1);
                Ok(())
            }
            Pattern::Assignment(pattern) => match pattern.left.as_ref() {
                Pattern::Identifier(identifier) => {
                    let value_reg = if let Some(init) = &declarator.init {
                        self.compile_expression(init)?
                    } else {
                        self.compile_expression(&pattern.right)?
                    };
                    self.declare_name_binding(&identifier.name, value_reg)?;
                    self.temp_top = value_reg.saturating_sub(1);
                    Ok(())
                }
                other => Err(CodegenError::Unsupported {
                    feature: "complex variable pattern",
                    span: other.span(),
                }),
            },
            other => Err(CodegenError::Unsupported {
                feature: "destructuring declarations",
                span: other.span(),
            }),
        }
    }
}
