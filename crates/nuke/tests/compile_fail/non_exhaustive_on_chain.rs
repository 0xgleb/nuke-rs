//! Compile-fail: forgetting an `.on(...)` for one of the subjects in
//! the reactor's list should prevent calling `.exhaustive()`. The
//! `Never` tail is what makes exhaustiveness a compile-time check.

use nuke::evm::{DecodeError, RawLog, SubscriptionSpec};
use nuke::prelude::*;
use nuke::Subject;

struct PoolA;
impl Subject for PoolA {
    type Id = String;
    type Event = ();
    const NAME: &'static str = "a";
    const SCHEMA_VERSION: u64 = 1;
    fn address() -> nuke::reexports::alloy_primitives::Address {
        nuke::reexports::alloy_primitives::Address::ZERO
    }
    fn subscription() -> SubscriptionSpec {
        SubscriptionSpec::logs_for(
            nuke::reexports::alloy_primitives::Address::ZERO,
            nuke::reexports::alloy_primitives::B256::ZERO,
        )
    }
    fn decode(_log: &RawLog) -> Result<Self::Event, DecodeError> {
        Ok(())
    }
}

struct PoolB;
impl Subject for PoolB {
    type Id = String;
    type Event = ();
    const NAME: &'static str = "b";
    const SCHEMA_VERSION: u64 = 1;
    fn address() -> nuke::reexports::alloy_primitives::Address {
        nuke::reexports::alloy_primitives::Address::ZERO
    }
    fn subscription() -> SubscriptionSpec {
        SubscriptionSpec::logs_for(
            nuke::reexports::alloy_primitives::Address::ZERO,
            nuke::reexports::alloy_primitives::B256::ZERO,
        )
    }
    fn decode(_log: &RawLog) -> Result<Self::Event, DecodeError> {
        Ok(())
    }
}

struct Bot;
subjects!(Bot, [PoolA, PoolB]);

fn main() {
    let event = <<Bot as Subscribed>::Subjects as HasSubject<PoolA>>::inject(
        "id".to_string(),
        (),
    );

    // ERROR: only one `.on(...)` for two subjects — `.exhaustive()`
    // is only available when the remaining tail is `Never`.
    let _fold = event.on(|_id, _ev| async move { 1 });
    // The next line cannot type-check because the remaining tail still
    // contains the unhandled `PoolB` subject.
    let _result = _fold.exhaustive();
}
