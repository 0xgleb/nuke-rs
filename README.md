# nuke-rs

A Tower-style, extensible framework for trading/crypto websocket
communication. **nuke-rs** reacts to market events and dispatches typed jobs
through apalis. The name comes from the design intent: a nuclear reactor
that *reacts* to market events and *triggers* downstream chain reactions.

## Status

Pre-alpha. The first cut targets EVM JSON-RPC websocket subscriptions
(`eth_subscribe("logs", ...)`) with a DEX/DEX arbitrage bot as the driving
example. CEX adapters, multi-chain support, reconnect/backpressure, and ABI
schema reconciliation will land in subsequent PRs.

## Design

Two user-facing traits, both leveraging the type system + a small
declarative macro for compile-time exhaustiveness — patterns mirrored from
[`event-sorcery`](https://github.com/0xgleb/st0x.liquidity):

- **`Subject`** — something on a chain that emits typed events. Each pool /
  contract is its own type, with `ADDRESS` as a `const` of the type.
- **`Reactor`** — what reacts to events from a *list* of subjects. The event
  type is *computed* from the subject list (no manual enum or `From` impls).
  Handle each subject via a `.on(...).on(...).exhaustive()` chain — forgetting
  a subject is a compile error.

The `subjects!` macro declares a reactor's subject list once and generates
the `Subscribed` and `HasSubject` impls.

## Example

See [`examples/arb_bot/main.rs`](examples/arb_bot/main.rs) — Uniswap V2 vs
SushiSwap V2 arbitrage detection on a single token pair.

```sh
nix develop --impure -c cargo run --example arb_bot
```

The end-to-end test (`tests/arb_bot_e2e.rs`) runs the same wiring against an
embedded mock JSON-RPC websocket server with a deterministic fixture — no
network, no secrets.

## Secrets

Live runs require an Ethereum websocket RPC URL. Declared in
[`secretspec.toml`](secretspec.toml) and loaded at runtime per the
[secretspec quick-start](https://secretspec.dev/quick-start/).

## Develop

```sh
direnv allow
cargo build --all-targets --locked
cargo test --all-targets --locked
```
