//! Compile-fail: forgetting an `.on(...)` for one of the deps in
//! the reactor's list should prevent calling `.exhaustive()`. The
//! `Never` tail is what makes exhaustiveness a compile-time check.

use nuke::Subject;
use nuke::prelude::*;

struct DepA;
impl Subject for DepA {
    type Id = String;
    type Event = ();
    const NAME: &'static str = "a";
    const SCHEMA_VERSION: u64 = 1;
}

struct DepB;
impl Subject for DepB {
    type Id = String;
    type Event = ();
    const NAME: &'static str = "b";
    const SCHEMA_VERSION: u64 = 1;
}

struct Bot;
deps!(Bot, [DepA, DepB]);

fn main() {
    let event = <<Bot as Dependent>::Deps as HasDep<DepA>>::inject("id".to_string(), ());

    // ERROR: only one `.on(...)` for two deps - `.exhaustive()` is only
    // available when the remaining tail is `Never`.
    let _fold = event.on(|_id, _ev| async move { 1 });
    // The next line cannot type-check because the remaining tail still
    // contains the unhandled `DepB` dep.
    let _result = _fold.exhaustive();
}
