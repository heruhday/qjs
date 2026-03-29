// Optimization Patch for Arithmetic Handlers

pub fn add_f64(a: f64, b: f64) -> f64 {
    a + b // Directly writing result to registers
}

pub fn sub_f64(a: f64, b: f64) -> f64 {
    a - b // Directly writing result to registers
}

pub fn mul_f64(a: f64, b: f64) -> f64 {
    a * b // Directly writing result to registers
}

pub fn lt_f64(a: f64, b: f64) -> bool {
    a < b // Directly using operations without boxing
}

// Slow path for type coercion
// (Other necessary code should remain intact or be defined elsewhere)