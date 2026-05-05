# nuke-rs - agent guidance

Durable architectural reference for any agent (or human) working in
this repo. Read it before changing the framework shape.

## What this project is

A general-purpose, event-driven framework for systems that:

1. Listen to long-running external event sources (websocket
   subscriptions or polling adapters).
2. React to those events (and their own internally-sourced events
   from a cqrs/es store) with reactors that *enqueue apalis jobs
   forming a DAG workflow*.
3. Talk to external services (POST requests, signed transactions,
   message sends, etc.) *only inside apalis jobs*, so durability,
   retries, and backoff come for free.

A `policy!` block written in the embedded DSL *compiles* into the
core of that DAG: predicate evaluation = computed steps, side-effect
points = jobs, control flow (`Given`/`All`/`Any`/`RejectIf`/
`EscalateIf`) = DAG edges. The output plugs straight into
`apalis_workflow::DagFlow`.

## What this project is NOT

- Not a trading-only framework. Not a crypto-only framework.
- Not an EVM/SVM/CEX framework. None of those names appear in any
  framework crate. They live *only* in `examples/*` and
  `adapters/*` crates that consume the framework.
- Not a thin wrapper around apalis or cqrs-es. The framework's
  contribution is the *composition*: typed-AST policies that compile
  to apalis DAGs, cqrs/es-backed reactors, the Source/TradingVenue
  abstractions that keep venue specifics out of core.

If you find yourself adding `alloy`, `ethers`, `web3`, an exchange
client, or any chain-specific type to a framework crate, *stop*.
That code belongs in an adapter crate or example.

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

| Crate | Role | Carries venue specifics? |
| --- | --- | --- |
| `nuke` | `Source<E>` / `Sink<T>` / `TradingVenue<...>` traits, the run loop, type-level subject/list machinery, eDSL, persist, job traits | No |
| `nuke-derive` | Framework proc-macros (e.g. `#[derive(Domain)]`) | No |
| `adapters/evm/` | EVM ws + signed-tx adapter (consumes the framework traits) | Yes (in this crate only) |
| `adapters/evm-derive/` | `#[derive(EvmSubject)]` proc-macro | Yes (in this crate only) |
| `examples/dex_arb/` | Cross-DEX arbitrage example using the EVM adapter + the framework | Yes (in this crate only) |
| `examples/<second>/` | Second example exercising polling + a non-DEX domain | Yes (in this crate only) |

The split into smaller framework crates (nuke-domain, nuke-policy,
nuke-job, nuke-persist) is a future decomposition once the
abstractions are stable. For now they live as modules inside `nuke`.

## Hard invariants

These are non-negotiable:

- *No `f64` in domain code.* Money / quantity / price / notional /
  spread / edge: all `rust_decimal::Decimal`. Newtypes per primitive.
- *No closures in the eDSL.* Every comparison, arithmetic op, field
  access, and combinator is a typed AST node with a name. The
  `policy!` macro must syntactically reject free-form Rust blocks.
- *No venue specifics in framework crates.* EVM/SVM/CEX live in
  `examples/*` and `adapters/*` only.
- *External-service side-effects only in jobs.* Reactors enqueue;
  jobs perform. This is what makes durability + retries free.
- *Apalis is internal, never exposed.* The public API stays in nuke
  vocab (`Source`, `Reactor`, `Job<Ctx>`, `policy!`). Apalis types
  (`WorkerBuilder`, `MemoryStorage`, `DagFlow`) are implementation
  details.
- *ASCII only.* No em-dashes, smart quotes, mathematical symbols,
  arrows, or bullets in docs / comments / error messages / strings.
  Substitute with ASCII equivalents (`-`, `"`, `<=`, `->`, `*`).
- *Prefer FP over mutable imperative.* Iterator chains, stream
  combinators, `try_collect`, `fold`, `try_fold` over `Vec::new()` +
  `for`-push patterns. Reach for `itertools` when stdlib iterators
  don't have what you need.
- *Examples ARE e2e tests.* No `examples/<name>/tests/` subdir. Each
  example crate has `src/lib.rs` (wiring) + `src/main.rs` (runner)
  with `#[cfg(test)]` modules in the lib for assertions / mocks. Run
  with `cargo test -p <name>`.

## Reference repos

These are *peer* frameworks the user uses. Read them before
designing analogous components here.

- `~/code/st0x/st0x.liquidity/crates/event-sorcery/` - the
  cqrs/es-on-cqrs-es adapter pattern (rich associated types, naming
  asymmetry `originate`/`evolve` vs `initialize`/`transition`,
  typed aggregate IDs, schema reconciliation). Our `nuke::persist`
  mirrors this.
- `~/code/st0x/st0x.liquidity/src/conductor/job.rs` - the canonical
  `Job<Ctx>` trait pattern with apalis + backon retries. The
  framework's job module mirrors this.
- `~/code/0xgleb/barter-rs/` - open-source Rust framework for
  event-driven live-trading + backtesting. Workspace split worth
  studying. Borrow `Validator`, `Processor<Event>`,
  `RiskApproved<T>` / `RiskRefused<T, Reason>` patterns. Consider
  `barter-integration` as a transport dep if the fit is clean.

## Workflow

- *Plain `git`, never Graphite.* No `gt` commands here, including
  `gt ls` (which silently initializes Graphite metadata).
- *One bash command per call.* Never chain commands with `&&`,
  `;`, or `||`, or decorative `echo "---"` separators. Parallel Bash
  tool calls instead.
- *Commit and push as you go.* Small, incremental commits per task.
  Push after each commit so the remote stays current. Don't
  accumulate uncommitted or unpushed work across multiple tasks.
  Keep `cargo test --workspace --all-targets`, `cargo clippy
  --workspace --all-targets -- -D warnings`, and `nix flake check`
  green at each commit.
- *Pre-commit hooks* (`rustfmt`, `nixfmt`, `taplo`, `actionlint`)
  run automatically and reformat files. After a hook reformat,
  re-stage the changed files and commit again. Never amend.
- *Documentation is part of the work.* Update `README.md`,
  `ROADMAP.md`, this file, per-crate READMEs, and module-level `//!`
  docs in the same commit as the code change. Stale docs are read as
  ground truth by the next contributor.

## Repo entry points

- `README.md` - what this is, how to use it, where the example lives.
- `ROADMAP.md` - epic-based plan, ordered by priority. First epic is
  always the next thing to implement.
- `docs/architecture.md` - long-form architecture reference with
  diagrams.
- `examples/dex_arb/src/main.rs` - the canonical worked example
  (cross-DEX arb with profit check + Job-based execution).
- `secretspec.toml` - secrets the example needs. Loaded via
  `secretspec_derive::declare_secrets!`.
