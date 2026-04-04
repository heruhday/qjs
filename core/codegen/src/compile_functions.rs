use super::*;

impl Codegen {
    pub(crate) fn compile_pending_functions(&mut self) -> Result<(), CodegenError> {
        while let Some(pending) = self.pending_functions.pop_front() {
            let entry_pc = self.builder.len();
            self.function_patches.push((
                pending.const_index,
                entry_pc,
                pending_function_is_async(&pending.body),
            ));

            let saved_temp_top = self.temp_top;
            let saved_fast_name_regs = self.fast_name_regs.clone();
            let saved_fast_name_runtime_slots = self.fast_name_runtime_slots.clone();
            let saved_fast_name_scope_stack = self.fast_name_scope_stack.clone();
            let saved_fast_name_scope_runtime_stack = self.fast_name_scope_runtime_stack.clone();
            let saved_nested_scope_depth = self.nested_scope_depth;
            let saved_fast_name_bindings_enabled = self.fast_name_bindings_enabled;
            let saved_current_self_upvalue = self.current_self_upvalue.clone();
            let saved_current_self_upvalue_reg = self.current_self_upvalue_reg;
            let saved_control_stack = self.control_stack.clone();
            let saved_label_stack = self.label_stack.clone();
            let saved_finally_stack = self.finally_stack.clone();
            let saved_finally_escape_patch_stack = self.finally_escape_patch_stack.clone();
            let saved_next_deferred_jump_id = self.next_deferred_jump_id;
            let saved_jump_sinks = self.jump_sinks.clone();
            self.temp_top = 0;
            self.fast_name_regs.clear();
            self.fast_name_runtime_slots.clear();
            self.fast_name_scope_stack.clear();
            self.fast_name_scope_runtime_stack.clear();
            self.nested_scope_depth = 0;
            self.fast_name_bindings_enabled = true;
            self.control_stack.clear();
            self.label_stack.clear();
            self.finally_stack.clear();
            self.finally_escape_patch_stack.clear();
            self.next_deferred_jump_id = 1;
            self.jump_sinks.clear();
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
            self.control_stack = saved_control_stack;
            self.label_stack = saved_label_stack;
            self.finally_stack = saved_finally_stack;
            self.finally_escape_patch_stack = saved_finally_escape_patch_stack;
            self.next_deferred_jump_id = saved_next_deferred_jump_id;
            self.jump_sinks = saved_jump_sinks;
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
                Pattern::Rest(pattern) => match pattern.argument.as_ref() {
                    Pattern::Identifier(identifier) => {
                        let reg = self.alloc_temp(Some(pattern.span))?;
                        self.builder.emit_load_rest_args(reg, index as u8);
                        self.declare_name_binding(&identifier.name, reg)?;
                        self.temp_top = reg.saturating_sub(1);
                    }
                    other => {
                        return Err(CodegenError::Unsupported {
                            feature: "complex rest parameter",
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

    pub(crate) fn compile_class_declaration(
        &mut self,
        class: &ast::Class,
    ) -> Result<(), CodegenError> {
        let Some(identifier) = &class.id else {
            return Err(CodegenError::Unsupported {
                feature: "anonymous class declaration",
                span: class.span,
            });
        };

        // Find the constructor method
        let constructor_func = class.body.iter().find_map(|element| {
            if let ast::ClassElement::Method(m) = element {
                if m.kind == ast::MethodKind::Constructor {
                    return Some(m.value.clone());
                }
            }
            None
        });

        // Use the constructor or create a default one
        let func = if let Some(mut ctor) = constructor_func {
            // Update constructor id to use the class name
            ctor.id = Some(identifier.clone());
            ctor
        } else {
            // Create a default constructor with the class name
            ast::Function {
                id: Some(identifier.clone()),
                params: vec![],
                body: ast::BlockStatement {
                    body: vec![],
                    span: class.span,
                },
                is_async: false,
                is_generator: false,
                span: class.span,
            }
        };

        // Compile the constructor as a function declaration
        self.compile_function_declaration(&func)?;

        // Create and set the prototype property on the constructor
        let class_reg = self.alloc_temp(Some(class.span))?;
        let proto_obj_reg = self.alloc_temp(Some(class.span))?;
        let proto_prop_slot = self.property_slot("prototype")?;
        let class_slot = self.name_slot(&identifier.name)?;
        
        // Load the class/constructor function
        self.builder.emit_load_name(class_reg, class_slot);
        
        // Create an empty object for the prototype
        self.builder.emit_new_obj(proto_obj_reg);
        
        // Set the prototype property on the constructor
        self.builder.emit_set_prop_ic(proto_obj_reg, class_reg, proto_prop_slot);
        
        self.temp_top = class_reg.saturating_sub(1);

        // Compile methods and assign them to the constructor's prototype
        for element in &class.body {
            if let ast::ClassElement::Method(method) = element {
                if method.kind != ast::MethodKind::Constructor {
                    self.compile_class_method(identifier, method)?;
                }
            }
        }

        // Compile fields
        for element in &class.body {
            if let ast::ClassElement::Field(field) = element {
                self.compile_class_field(identifier, field)?;
            }
        }

        // Compile static blocks and execute them
        for element in &class.body {
            if let ast::ClassElement::StaticBlock(block) = element {
                self.compile_static_block(identifier, block)?;
            }
        }

        Ok(())
    }

    fn compile_class_method(
        &mut self,
        class_name: &ast::Identifier,
        method: &ast::ClassMethod,
    ) -> Result<(), CodegenError> {
        // Allocate registers for the operation
        let class_reg = self.alloc_temp(Some(method.span))?;
        let method_func_reg = self.alloc_temp(Some(method.span))?;
        let proto_reg = self.alloc_temp(Some(method.span))?;

        // Load the class/constructor function  
        let class_slot = self.name_slot(&class_name.name)?;
        self.builder.emit_load_name(class_reg, class_slot);

        // Get the method name from the method's key
        let method_name = match &method.key {
            ast::PropertyKey::Identifier(ident) => ident.name.clone(),
            ast::PropertyKey::String(s) => s.value.clone(),
            _ => {
                return Err(CodegenError::Unsupported {
                    feature: "computed class method names",
                    span: method.span,
                });
            }
        };

        // Create a function for the method
        let mut method_function = method.value.clone();
        method_function.id = None; // Methods are typically anonymous

        // Compile the method function
        let const_index = self.reserve_function_constant();
        self.pending_functions.push_back(PendingFunction {
            const_index,
            body: PendingFunctionBody::Function(method_function),
        });
        self.builder.emit_new_func(method_func_reg, const_index);

        if method.is_static {
            // For static methods, assign directly to the class
            let method_prop_slot = self.property_slot(&method_name)?;
            self.builder.emit_set_prop_ic(method_func_reg, class_reg, method_prop_slot);
        } else {
            // For instance methods, assign to the prototype
            let proto_prop_slot = self.property_slot("prototype")?;
            self.builder.emit_get_prop_ic(proto_reg, class_reg, proto_prop_slot);

            let method_prop_slot = self.property_slot(&method_name)?;
            self.builder.emit_set_prop_ic(method_func_reg, proto_reg, method_prop_slot);
        }

        self.temp_top = class_reg.saturating_sub(1);
        Ok(())
    }

    fn compile_class_field(
        &mut self,
        class_name: &ast::Identifier,
        field: &ast::ClassField,
    ) -> Result<(), CodegenError> {
        // For now, skip accessors and private instance fields
        if field.is_accessor || (!field.is_static && matches!(field.key, ast::PropertyKey::PrivateName(_))) {
            return Ok(());
        }

        // For static fields, set them on the class
        if field.is_static {
            if let Some(value) = &field.value {
                let class_reg = self.alloc_temp(Some(field.span))?;
                let value_reg = self.compile_expression(value)?;
                let class_slot = self.name_slot(&class_name.name)?;

                self.builder.emit_load_name(class_reg, class_slot);

                match &field.key {
                    ast::PropertyKey::Identifier(ident) => {
                        let prop_slot = self.property_slot(&ident.name)?;
                        self.builder.emit_set_prop_ic(value_reg, class_reg, prop_slot);
                    }
                    ast::PropertyKey::String(s) => {
                        let prop_slot = self.property_slot(&s.value)?;
                        self.builder.emit_set_prop_ic(value_reg, class_reg, prop_slot);
                    }
                    ast::PropertyKey::PrivateName(ident) => {
                        // For private static fields, we can now set them using the private property opcodes
                        let slot = self.private_property_slot(&ident.name)?;
                        self.builder.emit_set_private_prop(value_reg, class_reg, slot);
                    }
                    _ => {
                        return Err(CodegenError::Unsupported {
                            feature: "computed static field names",
                            span: field.span,
                        });
                    }
                }

                self.temp_top = class_reg.saturating_sub(1);
            }
        }
        // Instance fields would be handled in the constructor, but that's complex
        // For now, we'll skip instance fields

        Ok(())
    }

    fn compile_static_block(
        &mut self,
        class_name: &ast::Identifier,
        block: &ast::BlockStatement,
    ) -> Result<(), CodegenError> {
        // Create a function that wraps the static block
        let static_block_func = ast::Function {
            id: None,
            params: vec![],
            body: block.clone(),
            is_async: false,
            is_generator: false,
            span: block.span,
        };

        // Compile the static block as a function
        let const_index = self.reserve_function_constant();
        self.pending_functions.push_back(PendingFunction {
            const_index,
            body: PendingFunctionBody::Function(static_block_func),
        });

        // Allocate registers
        let func_reg = self.alloc_temp(Some(block.span))?;
        let class_reg = self.alloc_temp(Some(block.span))?;
        let result_reg = self.alloc_temp(Some(block.span))?;

        // Load the class and create the function
        let class_slot = self.name_slot(&class_name.name)?;
        self.builder.emit_load_name(class_reg, class_slot);
        self.builder.emit_new_func(func_reg, const_index);

        // Call the static block function with the class as 'this'
        self.builder.emit_call_this(func_reg, class_reg, 0);
        self.builder.emit_mov(result_reg, ACC);

        self.temp_top = class_reg.saturating_sub(1);
        Ok(())
    }
}
