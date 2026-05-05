//! Compile-fail: comparing `Expr<DecT>` against `Expr<BoolT>` should
//! be rejected at compile time. The phantom type on `Expr<T>` carries
//! the value type so cross-type comparisons can't even be expressed.

use nuke::policy::ast::{BoolT, DecT, Expr, eq};

fn main() {
    let lhs = Expr::<DecT>::lit(rust_decimal::Decimal::from(1));
    let rhs = Expr::<BoolT>::lit(true);
    // ERROR: `eq` requires both sides to share the same `T`.
    let _ = eq(lhs, rhs);
}
