# nuke-rs

A general-purpose, event-driven Rust framework for systems that listen to
long-running event sources, react to events with durable DAG workflows, and talk
to external services through retryable jobs.

The core composition: **`Source` → cqrs/es → `Reactor` → apalis Job DAG →
`TradingVenue`**. A `policy!` block written in the embedded DSL **compiles**
into the core of that DAG.

## Status

Pre-alpha. The bootstrap PR landed the eDSL + backends + apalis runtime +
cqrs/es persistence + a DEX/DEX arbitrage example. The framework is currently
being refactored to make EVM-specific code live only in examples / adapter
crates — see [ROADMAP.md](ROADMAP.md).

## What it isn't

nuke-rs is **not** a trading-only or crypto-only framework. The framework crates
carry no blockchain / exchange specifics. EVM, SVM, CEX adapters live
exclusively in `examples/*` and `adapters/*`. The framework knows only abstract
traits (`Source<E>`, `TradingVenue<...>`, `Job<Ctx>`, `Reactor`, `EventSourced`)
plus the eDSL and the apalis + cqrs-es runtime substrate.

## Architecture in one diagram

```
External Sources (ws / poll)
       ↓ events
cqrs/es event store + Reactor
       ↓ enqueue Job<Ctx> instances
Apalis DAG Workflow (durable + retryable)
       ↓ POST / signed-tx / message-send
External Services (TradingVenue impls)
```

Long form with diagrams in [docs/architecture.md](docs/architecture.md).

## Design

Two user-facing traits + one declarative macro per layer, leaning hard on the
type system to make wrong wirings into compile errors.

| Layer       | User writes                                        | Framework provides                                                       |
| ----------- | -------------------------------------------------- | ------------------------------------------------------------------------ |
| Source      | An adapter impl OR uses a provided ws/poll adapter | `Source<E>` trait + ws/poll adapters                                     |
| Persistence | An `EventSourced` impl                             | `Lifecycle<E>`, cqrs-es bridge, schema reconciler                        |
| Reactor     | `Reactor::react` returning a Job DAG               | `Subject` / `Subscribed` / `subjects!` machinery, run loop               |
| Policy      | `policy! { ... }` blocks                           | Typed `Expr<T>` + `RuleNode` AST, 11 backends, the policy → DAG compiler |
| Jobs        | A `Job<Ctx>` impl per side-effect                  | `Job<Ctx>` trait, `work::<Ctx, J>` apalis handler, retries via `backon`  |
| Venue       | A `TradingVenue<...>` impl                         | `TradingVenue` trait, paper-trading sandbox                              |

## eDSL backends

Eleven folds over the same `RuleNode` AST: runtime evaluator, markdown digest,
JSON Schema, mermaid flowchart, SMT-LIB export (Z3/CVC5), proptest scaffolding,
SQL `WHERE` compiler, CBOR wire format with SHA-256 schema hash, semantic diff
(narrowed/widened/unrelated), coverage telemetry, TLA+ predicate export. Each is
independently usable.

## Example

[`examples/arb_bot/main.rs`](examples/arb_bot/main.rs) — DEX/DEX arbitrage
between Uniswap V2 and SushiSwap V2 on Ethereum mainnet. Demonstrates: a
chain-event Source via the EVM adapter, the detector reactor, a `policy!` block
that gates trade submission on profitability, and apalis Jobs that perform the
actual trade submission.

```sh
nix develop --impure -c cargo run --example arb_bot
```

The end-to-end test (`tests/arb_bot_e2e.rs`) runs the same wiring against an
embedded mock JSON-RPC websocket server with a deterministic fixture — no
network, no secrets, no flakes.

## Documentation entry points

- [CLAUDE.md](CLAUDE.md) — durable architectural reference for any agent (or
  human) working in this repo.
- [ROADMAP.md](ROADMAP.md) — epic-based plan, ordered by priority.
- [docs/architecture.md](docs/architecture.md) — long-form architecture
  reference with diagrams.
- Per-crate `README.md` files document each workspace member's role.

## Secrets

Live runs use `secretspec` ([quick-start](https://secretspec.dev/quick-start/),
[Rust SDK](https://secretspec.dev/sdk/rust/)). Declared in
[`secretspec.toml`](secretspec.toml).

## Develop

```sh
direnv allow
cargo build --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
nix flake check --impure --accept-flake-config
```

The `trybuild` compile-fail tests are `#[ignore]`d by default; run them manually
with `cargo test --test trybuild -- --ignored` (they snapshot rustc error
messages and drift across versions).
