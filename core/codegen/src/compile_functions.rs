use super::*;

impl Codegen {
    pub(crate) fn compile_pending_functions(&mut self) -> Result<(), CodegenError> {
        while let Some(pending) = self.pending_functions.pop_front() {
            let entry_pc = self.builder.len();
            self.function_patches.push((pending.const_index, entry_pc));

            let saved_temp_top = self.temp_top;
            let saved_fast_name_regs = self.fast_name_regs.clone();
            let saved_fast_name_runtime_slots = self.fast_name_runtime_slots.clone();
            let saved_fast_name_scope_stack = self.fast_name_scope_stack.clone();
            let saved_fast_name_scope_runtime_stack = self.fast_name_scope_runtime_stack.clone();
            let saved_nested_scope_depth = self.nested_scope_depth;
            let saved_fast_name_bindings_enabled = self.fast_name_bindings_enabled;
            let saved_current_self_upvalue = self.current_self_upvalue.clone();
            let saved_current_self_upvalue_reg = self.current_self_upvalue_reg;
            self.temp_top = 0;
            self.fast_name_regs.clear();
            self.fast_name_runtime_slots.clear();
            self.fast_name_scope_stack.clear();
            self.fast_name_scope_runtime_stack.clear();
            self.nested_scope_depth = 0;
            self.fast_name_bindings_enabled = true;
            let requires_runtime_env = pending_function_requires_runtime_env(&pending.body);
            self.enter_fast_name_scope_with_runtime_bindings(requires_runtime_env);
            self.current_self_upvalue = match &pending.body {
                PendingFunctionBody::Function(function) => function
                    .id
                    .as_ref()
                    .map(|identifier| (identifier.name.clone(), 0)),
                PendingFunctionBody::Arrow(_) => None,
            };
            self.current_self_upvalue_reg = None;

            if requires_runtime_env {
                let env_reg = self.alloc_temp(None)?;
                self.builder.emit_create_env(env_reg);
            }

            match pending.body {
                PendingFunctionBody::Function(function) => {
                    self.compile_function_params(&function.params)?;
                    self.compile_statement_list(&function.body.body, false)?;
                    self.builder.emit_ret_u();
                }
                PendingFunctionBody::Arrow(function) => {
                    self.compile_function_params(&function.params)?;
                    match function.body {
                        ArrowBody::Expression(expression) => {
                            let reg = self.compile_expression(&expression)?;
                            self.builder.emit_mov(ACC, reg);
                            self.temp_top = reg;
                            self.builder.emit_ret();
                        }
                        ArrowBody::Block(block) => {
                            self.compile_statement_list(&block.body, false)?;
                            self.builder.emit_ret_u();
                        }
                    }
                }
            }

            self.leave_fast_name_scope();
            self.temp_top = saved_temp_top;
            self.fast_name_regs = saved_fast_name_regs;
            self.fast_name_runtime_slots = saved_fast_name_runtime_slots;
            self.fast_name_scope_stack = saved_fast_name_scope_stack;
            self.fast_name_scope_runtime_stack = saved_fast_name_scope_runtime_stack;
            self.nested_scope_depth = saved_nested_scope_depth;
            self.fast_name_bindings_enabled = saved_fast_name_bindings_enabled;
            self.current_self_upvalue = saved_current_self_upvalue;
            self.current_self_upvalue_reg = saved_current_self_upvalue_reg;
        }

        Ok(())
    }

    pub(crate) fn compile_function_declaration(
        &mut self,
        function: &Function,
    ) -> Result<(), CodegenError> {
        let Some(identifier) = &function.id else {
            return Err(CodegenError::Unsupported {
                feature: "anonymous function declaration",
                span: function.span,
            });
        };
        let dst = self.alloc_temp(Some(function.span))?;
        let const_index = self.reserve_function_constant();
        self.pending_functions.push_back(PendingFunction {
            const_index,
            body: PendingFunctionBody::Function(function.clone()),
        });
        self.builder.emit_new_func(dst, const_index);
        self.builder.emit_set_upval(dst, 0);
        self.declare_name_binding(&identifier.name, dst)?;
        self.temp_top = dst.saturating_sub(1);
        Ok(())
    }

    pub(crate) fn compile_function_params(
        &mut self,
        params: &[Pattern],
    ) -> Result<(), CodegenError> {
        for (index, param) in params.iter().enumerate() {
            match param {
                Pattern::Identifier(identifier) => {
                    let reg = self.alloc_temp(Some(identifier.span))?;
                    self.builder.emit_load_arg(reg, index as u8);
                    self.declare_name_binding(&identifier.name, reg)?;
                    self.temp_top = reg.saturating_sub(1);
                }
                Pattern::Assignment(pattern) => match pattern.left.as_ref() {
                    Pattern::Identifier(identifier) => {
                        let reg = self.alloc_temp(Some(pattern.span))?;
                        self.builder.emit_load_arg(reg, index as u8);
                        let flag = self.alloc_temp(Some(pattern.span))?;
                        self.builder.emit_is_undef(flag, reg);
                        let skip_default = self.emit_placeholder_jmp_false(flag);
                        let default_reg = self.compile_expression(&pattern.right)?;
                        self.builder.emit_mov(reg, default_reg);
                        let after_default = self.builder.len();
                        self.patch_jump(
                            skip_default,
                            after_default,
                            JumpPatchKind::JmpFalse { reg: flag },
                        );
                        self.declare_name_binding(&identifier.name, reg)?;
                        self.temp_top = reg.saturating_sub(1);
                    }
                    other => {
                        return Err(CodegenError::Unsupported {
                            feature: "complex default parameter",
                            span: other.span(),
                        });
                    }
                },
                other => {
                    return Err(CodegenError::Unsupported {
                        feature: "complex function parameter",
                        span: other.span(),
                    });
                }
            }
        }
        Ok(())
    }
}
