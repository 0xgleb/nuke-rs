# nuke-rs - agent guidance

Durable architectural reference for any agent (or human) working in this repo.
Read it before changing the framework shape.

## What this project is

A general-purpose, event-driven framework for systems that:

1. Listen to long-running external event sources (websocket subscriptions or
   polling adapters).
2. React to those events (and their own internally-sourced events from a cqrs/es
   store) with reactors that _enqueue apalis jobs forming a DAG workflow_.
3. Talk to external services (POST requests, signed transactions, message sends,
   etc.) _only inside apalis jobs_, so durability, retries, and backoff come for
   free.

A `policy!` block written in the embedded DSL _compiles_ into the core of that
DAG: predicate evaluation = computed steps, side-effect points = jobs, control
flow (`Given`/`All`/`Any`/`RejectIf`/ `EscalateIf`) = DAG edges. The output
plugs straight into `apalis_workflow::DagFlow`.

## What this project is NOT

- Not a trading-only framework. Not a crypto-only framework.
- Not an EVM/SVM/CEX framework. None of those names appear in any framework
  crate. They live _only_ in `examples/*` and adapter crates (e.g.
  `crates/evm/`) and example crates that consume the framework.
- Not a thin wrapper around apalis or cqrs-es. The framework's contribution is
  the _composition_: typed-AST policies that compile to apalis DAGs,
  cqrs/es-backed reactors, the Source/TradingVenue abstractions that keep venue
  specifics out of core.

If you find yourself adding `alloy`, `ethers`, `web3`, an exchange client, or
any chain-specific type to a framework crate, _stop_. That code belongs in an
adapter crate or example.

## Three-layer architecture

```
+----------------------------------------------------------------+
|  External Event Sources (ws subscriptions OR polling)          |
|  via Source<E> trait - adapters per *transport*, not venue     |
+--------------------+-------------------------------------------+
                     | events
                     v
+----------------------------------------------------------------+
|  cqrs/es event store (nuke::persist, event-sorcery-style)      |
|  + Reactors that consume events and enqueue Job DAGs           |
+--------------------+-------------------------------------------+
                     | enqueue Job<Ctx> instances
                     v
+----------------------------------------------------------------+
|  Apalis DAG Workflow Execution                                 |
|  (durable storage + retries + backoff for free)                |
+--------------------+-------------------------------------------+
                     | POST / signed-tx / etc.
                     v
+----------------------------------------------------------------+
|  External Services via TradingVenue<...> impls                 |
|  (EVM RPC, CEX REST, paper-trading, ...)                       |
+----------------------------------------------------------------+
```

## Workspace layout (target)

| Crate                | Role                                                                                                                             | Carries venue specifics? |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| `nuke`               | `Source<E>` / `Sink<T>` / `TradingVenue<...>` traits, the run loop, type-level subject/list machinery, eDSL, persist, job traits | No                       |
| `nuke-derive`        | Framework proc-macros (e.g. `#[derive(Domain)]`)                                                                                 | No                       |
| `crates/evm/`        | EVM ws + signed-tx adapter (consumes the framework traits)                                                                       | Yes (in this crate only) |
| `crates/evm-derive/` | `#[derive(EvmSubject)]` proc-macro                                                                                               | Yes (in this crate only) |
| `examples/dex_arb/`  | Cross-DEX arbitrage example using the EVM adapter + the framework                                                                | Yes (in this crate only) |
| `examples/<second>/` | Second example exercising polling + a non-DEX domain                                                                             | Yes (in this crate only) |

The split into smaller framework crates (nuke-domain, nuke-policy, nuke-job,
nuke-persist) is a future decomposition once the abstractions are stable. For
now they live as modules inside `nuke`.

## Hard invariants

These are non-negotiable:

- _No `f64` in domain code._ Money / quantity / price / notional / spread /
  edge: all `rust_decimal::Decimal`. Newtypes per primitive.
- _No closures in the eDSL._ Every comparison, arithmetic op, field access, and
  combinator is a typed AST node with a name. The `policy!` macro must
  syntactically reject free-form Rust blocks.
- _No venue specifics in framework crates._ EVM/SVM/CEX live in adapter crates
  (e.g. `crates/evm/`) and `examples/*` only.
- _External-service side-effects only in jobs._ Reactors enqueue; jobs perform.
  This is what makes durability + retries free.
- _Apalis is internal, never exposed._ The public API stays in nuke vocab
  (`Source`, `Reactor`, `Job<Ctx>`, `policy!`). Apalis types (`WorkerBuilder`,
  `MemoryStorage`, `DagFlow`) are implementation details.
- _ASCII only._ No em-dashes, smart quotes, mathematical symbols, arrows, or
  bullets in docs / comments / error messages / strings. Substitute with ASCII
  equivalents (`-`, `"`, `<=`, `->`, `*`).
- _Prefer FP over mutable imperative._ Iterator chains, stream combinators,
  `try_collect`, `fold`, `try_fold` over `Vec::new()` + `for`-push patterns.
  Reach for `itertools` when stdlib iterators don't have what you need.
- _Examples ARE e2e tests._ No `examples/<name>/tests/` subdir. Each example
  crate has `src/lib.rs` (wiring) + `src/main.rs` (runner) with `#[cfg(test)]`
  modules in the lib for assertions / mocks. Run with `cargo test -p <name>`.

## Workflow

- _Plain `git`, never Graphite._ No `gt` commands here, including `gt ls` (which
  silently initializes Graphite metadata).
- _One bash command per call._ Never chain commands with `&&`, `;`, or `||`, or
  decorative `echo "---"` separators. Parallel Bash tool calls instead.
- _Commit and push as you go._ Small, incremental commits per task. Push after
  each commit so the remote stays current. Don't accumulate uncommitted or
  unpushed work across multiple tasks. Keep
  `cargo test --workspace --all-targets`,
  `cargo clippy
  --workspace --all-targets -- -D warnings`, and
  `nix flake check` green at each commit.
- _Pre-commit hooks_ (`rustfmt`, `nixfmt`, `taplo`, `actionlint`) run
  automatically and reformat files. After a hook reformat, re-stage the changed
  files and commit again. Never amend.
- _Documentation is part of the work._ Update `README.md`, `ROADMAP.md`, this
  file, per-crate READMEs, and module-level `//!` docs in the same commit as the
  code change. Stale docs are read as ground truth by the next contributor.

## Repo entry points

- `README.md` - what this is, how to use it, where the example lives.
- `ROADMAP.md` - epic-based plan, ordered by priority. First epic is always the
  next thing to implement.
- `docs/architecture.md` - long-form architecture reference with diagrams.
- `examples/dex_arb/src/main.rs` - the canonical worked example (cross-DEX arb
  with profit check + Job-based execution).
- `secretspec.toml` - secrets the example needs. Loaded via
  `secretspec_derive::declare_secrets!`.
