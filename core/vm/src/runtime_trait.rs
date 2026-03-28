#![allow(unused)]

/* ============================================================
   Arithmetic Operators
============================================================ */

pub trait ArithmeticOps {
    fn add(&self, rhs: &Self) -> Self; // +
    fn sub(&self, rhs: &Self) -> Self; // -
    fn mul(&self, rhs: &Self) -> Self; // *
    fn div(&self, rhs: &Self) -> Self; // /
    fn rem(&self, rhs: &Self) -> Self; // %

    fn pow(&self, rhs: &Self) -> Self; // **

    fn inc(&self) -> Self; // ++
    fn dec(&self) -> Self; // --

    fn unary_plus(&self) -> Self; // +x
    fn unary_minus(&self) -> Self; // -x
}

/* ============================================================
   Comparison Operators
============================================================ */

pub trait ComparisonOps {
    fn eq(&self, rhs: &Self) -> Self; // ==
    fn ne(&self, rhs: &Self) -> Self; // !=

    fn strict_eq(&self, rhs: &Self) -> Self; // ===
    fn strict_ne(&self, rhs: &Self) -> Self; // !==

    fn gt(&self, rhs: &Self) -> Self; // >
    fn lt(&self, rhs: &Self) -> Self; // <
    fn ge(&self, rhs: &Self) -> Self; // >=
    fn le(&self, rhs: &Self) -> Self; // <=
}

/* ============================================================
   Logical Operators
============================================================ */

pub trait LogicalOps {
    fn logical_and(&self, rhs: &Self) -> Self; // &&
    fn logical_or(&self, rhs: &Self) -> Self; // ||
    fn logical_not(&self) -> Self; // !
}

/* ============================================================
   Bitwise Operators
============================================================ */

pub trait BitwiseOps {
    fn bit_and(&self, rhs: &Self) -> Self; // &
    fn bit_or(&self, rhs: &Self) -> Self; // |
    fn bit_xor(&self, rhs: &Self) -> Self; // ^
    fn bit_not(&self) -> Self; // ~

    fn shl(&self, rhs: &Self) -> Self; // <<
    fn shr(&self, rhs: &Self) -> Self; // >>
    fn ushr(&self, rhs: &Self) -> Self; // >>>
}

/* ============================================================
   Assignment Operators
============================================================ */

pub trait AssignmentOps {
    fn assign(&mut self, rhs: Self); // =
    fn add_assign(&mut self, rhs: Self); // +=
    fn sub_assign(&mut self, rhs: Self); // -=
    fn mul_assign(&mut self, rhs: Self); // *=
    fn div_assign(&mut self, rhs: Self); // /=
    fn rem_assign(&mut self, rhs: Self); // %=

    fn pow_assign(&mut self, rhs: Self); // **=

    fn shl_assign(&mut self, rhs: Self); // <<=
    fn shr_assign(&mut self, rhs: Self); // >>=
    fn ushr_assign(&mut self, rhs: Self); // >>>=

    fn bit_and_assign(&mut self, rhs: Self); // &=
    fn bit_or_assign(&mut self, rhs: Self); // |=
    fn bit_xor_assign(&mut self, rhs: Self); // ^=
}

/* ============================================================
   Logical Assignment
============================================================ */

pub trait LogicalAssignOps {
    fn and_assign(&mut self, rhs: Self); // &&=
    fn or_assign(&mut self, rhs: Self); // ||=
}

/* ============================================================
   Nullish Operators
============================================================ */

pub trait NullishOps {
    fn nullish_coalesce(&self, rhs: &Self) -> Self; // ??
    fn nullish_assign(&mut self, rhs: Self); // ??=
}

/* ============================================================
   Type / Object Operators
============================================================ */

pub trait TypeOps {
    fn typeof_(&self) -> Self; // typeof
    fn instanceof(&self, rhs: &Self) -> Self; // instanceof
    fn in_(&self, rhs: &Self) -> Self; // in
    fn delete(&self) -> Self; // delete
}

/* ============================================================
   ECMAScript Conversion Operations
============================================================ */

pub trait CoercionOps {
    fn to_number(&self) -> Self;
    fn to_string(&self) -> Self;
    fn to_boolean(&self) -> Self;
    fn to_primitive(&self) -> Self;
}

/* ============================================================
   Property Access
============================================================ */

pub trait PropertyOps {
    fn get(&self, key: &Self) -> Self;

    fn set(&mut self, key: Self, value: Self);

    fn has(&self, key: &Self) -> Self;

    fn delete_property(&mut self, key: &Self) -> Self;
}

/* ============================================================
   Function Call / Construction => just prototype (TODO)
============================================================ */

pub trait CallOps: Sized {
    fn call(&self, this: &Self, args: &[Self]) -> Self;

    fn construct(&self, args: &[Self]) -> Self;
}

/* ============================================================
   Ternary Operator Helper
============================================================ */

pub trait Ternary {
    fn ternary(cond: &Self, a: &Self, b: &Self) -> Self;
}

/* ============================================================
   Master Trait (Full Runtime API)
============================================================ */

pub trait ValueOps:
    ArithmeticOps
    + ComparisonOps
    + LogicalOps
    + BitwiseOps
    + AssignmentOps
    + LogicalAssignOps
    + NullishOps
    + TypeOps
    + CoercionOps
    + PropertyOps
    + CallOps
    + Ternary
{
}
