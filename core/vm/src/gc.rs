pub use ::gc::*;

use crate::js_value::{JSValue, string_from_value};
use crate::vm::{JSObject, JSString, ObjectKind, PropertyKey, Shape, VM};

fn clear_gc_marks(vm: &mut VM) {
    for &shape_ptr in &vm.shapes {
        unsafe {
            (*shape_ptr).header.marked = false;
        }
    }

    for &obj_ptr in &vm.objects {
        unsafe {
            (*obj_ptr).header.marked = false;
        }
    }

    for &string_ptr in &vm.strings {
        unsafe {
            (*string_ptr).header.marked = false;
        }
    }
}

fn mark_property_key(vm: &mut VM, key: PropertyKey) {
    match key {
        PropertyKey::Atom(atom) | PropertyKey::PrivateName(atom) => {
            vm.atoms.mark(atom);
            let interned = {
                let text = vm.atoms.resolve(atom).to_owned();
                vm.interned_strings.get(&text).copied()
            };
            if let Some(value) = interned {
                mark_value(vm, value);
            }
        }
        PropertyKey::Value(value) => mark_value(vm, value),
        PropertyKey::Id(_) | PropertyKey::Index(_) => {}
    }
}

fn mark_shape(vm: &mut VM, shape_ptr: *mut Shape) {
    if shape_ptr.is_null() {
        return;
    }

    let (parent, prototype, proto_cache_shape, key) = unsafe {
        debug_assert_eq!((*shape_ptr).header.obj_type, ObjType::Shape);

        if (*shape_ptr).header.marked {
            return;
        }

        (*shape_ptr).header.marked = true;
        (
            (*shape_ptr).parent,
            (*shape_ptr).prototype,
            (*shape_ptr).proto_cache_shape,
            (*shape_ptr).key,
        )
    };

    if let Some(key) = key {
        mark_property_key(vm, key);
    }

    if let Some(parent) = parent {
        mark_shape(vm, parent);
    }

    if let Some(prototype) = prototype {
        mark_shape(vm, prototype);
    }

    if let Some(proto_cache_shape) = proto_cache_shape {
        mark_shape(vm, proto_cache_shape);
    }
}

fn mark_string(vm: &mut VM, string_ptr: *mut JSString) {
    if string_ptr.is_null() {
        return;
    }

    unsafe {
        debug_assert_eq!((*string_ptr).header.obj_type, ObjType::String);

        if (*string_ptr).header.marked {
            return;
        }

        (*string_ptr).header.marked = true;
        vm.atoms.mark((*string_ptr).atom);
    }
}

fn mark_object(vm: &mut VM, obj_ptr: *mut JSObject) {
    if obj_ptr.is_null() {
        return;
    }

    let (shape, properties, named_values, kind) = unsafe {
        debug_assert_eq!((*obj_ptr).header.obj_type, ObjType::Object);

        if (*obj_ptr).header.marked {
            return;
        }

        (*obj_ptr).header.marked = true;
        (
            (*obj_ptr).shape,
            (*obj_ptr)
                .properties
                .iter()
                .map(|(key, value)| (*key, *value))
                .collect::<Vec<_>>(),
            (*obj_ptr)
                .named_present
                .iter()
                .copied()
                .zip((*obj_ptr).named_values.iter().copied())
                .filter_map(|(present, value)| present.then_some(value))
                .collect::<Vec<_>>(),
            (*obj_ptr).kind.clone(),
        )
    };

    mark_shape(vm, shape);

    for (key, value) in properties {
        mark_property_key(vm, key);
        mark_value(vm, value);
    }

    for value in named_values {
        mark_value(vm, value);
    }

    match kind {
        ObjectKind::Ordinary(object) | ObjectKind::Env(object) => {
            object.trace_atoms(&mut vm.atoms);
            for value in object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::Array(array) => {
            array.object.trace_atoms(&mut vm.atoms);
            for value in array.elements {
                mark_value(vm, value);
            }
            if let Some(sparse) = array.sparse {
                for value in sparse.into_values() {
                    mark_value(vm, value);
                }
            }
            for value in array.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = array.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::BoolArray(array) => {
            array.object.trace_atoms(&mut vm.atoms);
            for value in array.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = array.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::Uint8Array(array) => {
            array.object.trace_atoms(&mut vm.atoms);
            for value in array.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = array.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::Int32Array(array) => {
            array.object.trace_atoms(&mut vm.atoms);
            for value in array.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = array.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::Float64Array(array) => {
            array.object.trace_atoms(&mut vm.atoms);
            for value in array.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = array.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::StringArray(array) => {
            array.object.trace_atoms(&mut vm.atoms);
            for value in array.elements {
                mark_value(vm, value);
            }
            for value in array.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = array.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::Iterator { values, .. } => {
            for value in values {
                mark_value(vm, value);
            }
        }
        ObjectKind::Function(function) => {
            if let Some(name) = function.name {
                vm.atoms.mark(name);
            }
            for param in function.params {
                vm.atoms.mark(param);
            }
            for value in function.body {
                mark_value(vm, value);
            }
            if let Some(prototype) = function.prototype {
                mark_value(vm, prototype);
            }
            mark_value(vm, function.descriptor);
            for value in function.upvalues {
                mark_value(vm, value);
            }
        }
        ObjectKind::Closure(closure) => {
            mark_value(vm, closure.function);
            for value in closure.captures {
                mark_value(vm, value);
            }
        }
        ObjectKind::NativeFunction(function) => {
            if let Some(name) = function.name {
                vm.atoms.mark(name);
            }
        }
        ObjectKind::NativeClosure(closure) => {
            if let Some(name) = closure.name {
                vm.atoms.mark(name);
            }
            for value in closure.captures {
                mark_value(vm, value);
            }
        }
        ObjectKind::Class(class) => {
            if let Some(name) = class.name {
                vm.atoms.mark(name);
            }
            if let Some(prototype) = class.prototype {
                mark_value(vm, prototype);
            }
            if let Some(constructor) = class.constructor {
                mark_value(vm, constructor);
            }
            mark_value(vm, class.base);
            for (atom, value) in class.static_props {
                vm.atoms.mark(atom);
                mark_value(vm, value);
            }
        }
        ObjectKind::Module(module) => {
            if let Some(name) = module.name {
                vm.atoms.mark(name);
            }
            for (atom, value) in module.exports {
                vm.atoms.mark(atom);
                mark_value(vm, value);
            }
        }
        ObjectKind::Instance(instance) => {
            mark_value(vm, instance.class);
            instance.object.trace_atoms(&mut vm.atoms);
            for value in instance.object.values {
                mark_value(vm, value);
            }
            if let Some(prototype) = instance.object.prototype {
                mark_value(vm, prototype);
            }
        }
        ObjectKind::Symbol(symbol) => {
            if let Some(description) = symbol.description {
                vm.atoms.mark(description);
            }
        }
    }
}

fn mark_value(vm: &mut VM, value: JSValue) {
    if let Some(ptr) = value.as_heap_ptr() {
        match unsafe { (*ptr).obj_type } {
            ObjType::Object => mark_object(vm, ptr as *mut JSObject),
            ObjType::String => mark_string(vm, ptr as *mut JSString),
            ObjType::Shape => mark_shape(vm, ptr as *mut Shape),
        }
    }
}

fn sweep_objects(vm: &mut VM) {
    vm.objects.retain(|obj_ptr| {
        let keep = unsafe { (**obj_ptr).header.marked };
        if !keep {
            unsafe {
                drop(Box::from_raw(*obj_ptr));
            }
        }
        keep
    });
}

fn sweep_shapes(vm: &mut VM) {
    vm.shapes.retain(|shape_ptr| {
        let keep = unsafe { (**shape_ptr).header.marked };
        if !keep {
            unsafe {
                drop(Box::from_raw(*shape_ptr));
            }
        }
        keep
    });
}

fn sweep_strings(vm: &mut VM) {
    vm.interned_strings.retain(|_, value| {
        string_from_value(*value)
            .map(|string_ptr| unsafe { (*string_ptr).header.marked })
            .unwrap_or(false)
    });

    vm.strings.retain(|string_ptr| {
        let keep = unsafe { (**string_ptr).header.marked };
        if !keep {
            unsafe {
                drop(Box::from_raw(*string_ptr));
            }
        }
        keep
    });
}

pub fn collect_garbage(vm: &mut VM) {
    clear_gc_marks(vm);

    let active_frames: Vec<_> = vm
        .frame
        .active_frames()
        .iter()
        .map(|frame| {
            (
                frame.regs,
                frame.inline_args,
                frame.args.clone(),
                frame.header.function_value,
                frame.header.env,
                frame.ic_vector.iter().map(|ic| ic.key).collect::<Vec<_>>(),
            )
        })
        .collect();

    for (regs, inline_args, args, function_value, env, ic_keys) in active_frames {
        for value in regs {
            mark_value(vm, value);
        }

        for value in inline_args {
            mark_value(vm, value);
        }

        for value in args {
            mark_value(vm, value);
        }

        if let Some(function_value) = function_value {
            mark_value(vm, function_value);
        }

        if let Some(env) = env {
            mark_value(vm, env);
        }

        for key in ic_keys.into_iter().flatten() {
            mark_property_key(vm, key);
        }
    }

    let const_pool = vm.const_pool.clone();
    for value in const_pool {
        mark_value(vm, value);
    }

    let globals: Vec<_> = vm.global_object.values().copied().collect();
    for value in globals {
        mark_value(vm, value);
    }

    let scopes = vm.scope_chain.clone();
    for value in scopes {
        mark_value(vm, value);
    }

    let upvalues = vm.upvalues.clone();
    for value in upvalues {
        mark_value(vm, value);
    }

    mark_value(vm, vm.last_exception);

    sweep_objects(vm);
    sweep_shapes(vm);
    sweep_strings(vm);
    vm.atoms.sweep();
}
