# nuke-derive

Proc-macros for the framework crate.

| Macro               | What it does                                                                                                                                                                                               |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[derive(Domain)]` | Generates typed accessors and a `read_field(&self, name: &str) -> Option<SlotValue>` method on a domain entity struct so the eDSL's `field::<T>(...)` constructors can address fields by name type-safely. |

Used internally by the eDSL's typed expression layer. Adopters who write their
own domain types reach for this derive to integrate with the policy compiler.
