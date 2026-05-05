# event-sorcery

Ergonomic typed wrapper around [`cqrs-es`][cqrs-es], plus the foundational
type-level dependency-list machinery the rest of the workspace builds on.

| Module          | What it exports                                                                                                                                                                                           |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dep`           | `Dep` (minimal "I have an `Id` and an `Event`" trait), `Cons` / `Nil` / `Never` cells, `OneOf` / `Fold` discriminated-union chain, `Dependent` / `DepList` / `HasDep`, `deps!` / `register_deps!` macros. |
| `event_sourced` | `EventSourced` - user-facing trait with rich associated types and consts; replaces direct `cqrs_es::Aggregate` usage.                                                                                     |
| `lifecycle`     | `Lifecycle<E>` - blanket `cqrs_es::Aggregate` impl bridging `EventSourced` to cqrs-es. Adopters never touch cqrs-es directly.                                                                             |

## Why two responsibilities in one crate?

`Dep` and the dep-list machinery were originally inside the framework crate.
Pulling them down here gives the same `deps!` idiom to both external-stream deps
(a `nuke::Subject`) and internal aggregates (an `EventSourced`). Reactor authors
write the same macro regardless of what they're depending on.

## Why not just use cqrs-es directly?

The `Aggregate` trait has sharp edges that have caused production bugs:
infallible `apply` (financial code can't panic on overflow), stringly-typed
aggregate IDs, no schema versioning, flat command handling. `EventSourced` fixes
those one-by-one - typed IDs, fallible state transitions, `SCHEMA_VERSION`
const, separate `originate`/`evolve` (event-side) vs `initialize`/`transition`
(command-side) methods. `Lifecycle<E>` provides the `cqrs_es::Aggregate` blanket
impl so adopters never see cqrs-es internals.

[cqrs-es]: https://crates.io/crates/cqrs-es
