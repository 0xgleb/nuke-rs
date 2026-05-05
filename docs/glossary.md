# Glossary

Definitions for nuke-rs's vocabulary. Authoritative; the source-tree docstrings
should agree with this and be a touchstone when they don't. Ordered roughly by
where the concept appears in the run loop: external sources -> bus -> reactor ->
jobs -> external services.

## External-source layer

### Dep

Anything a reactor reacts to. Has a typed `Id` and a typed `Event`. Implemented
by everything that can sit in a reactor's
[`Dependent::Deps`](#dependentdependents) list - a [Subject](#subject) for an
external stream, an `EventSourced` aggregate for an internal event store. Lives
in `event-sorcery::dep`.

### Subject

A typed marker for one external-stream Dep. Refines `Dep` with two consts
(`NAME` for stable telemetry / audit, `SCHEMA_VERSION` for the schema
reconciler) plus the `Debug + Display + Clone` bounds the framework relies on
for routing. Adapter crates extend Subject with the I/O methods their transport
needs (e.g. `evm::EvmSubject` adds `address()` / `subscription()` / `decode()`).

### Dependent / Deps

`trait Dependent { type Deps: DepList }` is what a reactor implements (usually
via the `deps!` macro) to declare its compile-time list of Deps. The list
determines the reactor's input event union via [`DepList::Event`](#deplist).

### DepList

Computed event union over a `Cons<H, Cons<...>>`-shaped list of Deps. For
`Cons<A, Cons<B, Nil>>`, `DepList::Event` is
`OneOf<(A::Id, A::Event), OneOf<(B::Id, B::Event), Never>>` - the `Never` tail
is what makes the [`OneOf`](#oneof-fold) `.exhaustive()` chain a compile-time
check.

### OneOf / Fold

The discriminated event union DepList computes plus the
`.on(...).exhaustive().await` chain reactors use to handle it. Lives in
`event-sorcery::dep`.

### Source / Transport / Wire

- A _Transport_ is a long-lived handle (a ws connection, an HTTP client, an
  in-process bus) that knows how to "open" Deps. Adapters implement `Transport`
  once per transport type, naming the per-target-list accumulator via
  `Transport::Out<L>`.
- `Wire<D>` is the per-dep wiring step - one impl per (transport, dep family).
  Body: open one subscription / register one decoder / schedule one polling
  task. The framework's `Subscribe<L, T>` walker in `nuke::subscribe` recurses
  over the dep list and calls `Wire<H>::wire` for each.

### ExtStream / ExtQuery / PollingLayer

Tower-shaped adapter primitives in `nuke::ext`:

- `ExtStream` - long-running typed event source.
  `fn stream(self) ->
  impl Stream<Item = Result<Event, Error>>`.
- `ExtQuery` - typed request/response.
  `fn query(&self, req) -> impl
  Future<Output = Result<Resp, Error>>`.
- `Polling<Q, R, S>` - auto-impls `ExtStream` for any `ExtQuery` plus a
  `Stream<Item = ()>` schedule. The load-bearing piece: a polling adapter is "an
  `ExtQuery` plus a tick stream", never hand-written.

ExtStream is the next-generation framing of Subject; the two will converge once
the run loop's Reactor wiring is updated to consume `ExtStream`s directly (see
[`#81`](../ROADMAP.md)).

## Reactor layer

### Reactor

What reacts to events from a [Dep list](#dependentdependents). Pure decider: the
typed `react(event) -> Vec<Self::Job>` returns the work the framework should
enqueue. Reactors do not perform work directly.

### Job\<Ctx\>

Unit of durable retryable work. A serializable struct with `label() -> Label`
plus `perform(&self, &Ctx) -> Result<(), Error>`. Apalis owns durability,
retries (via `backon`), and backoff. Every external-service interaction (POST /
signed-tx / venue `place_trade` / message-send) lives inside a Job's `perform`
so those guarantees come for free.

## Policy layer

### Policy / RuleNode\<A\>

Verdict logic plus side-effect verbs. `RuleNode<A>` is the typed AST, generic
over the action type `A` (default `()` for action-free verdict-only rules).
Variants: `Given` (gate), `RejectIf` / `EscalateIf` (typed verdicts), `All` /
`Any` (combinators), `Bind` (let-binding), `Do(A)` (side-effect leaf).

### Decision

Total three-way verdict: `Allow | Deny | Escalate`. Returned by the runtime
evaluator. `Deny` and `Escalate` carry the structured [`Reason`](#reason) and
captured [`Bindings`](#bindings) so the verdict is self-explanatory and
reproducible.

### Reason

Typed format-string AST for verdict explanations. Templates have named slots;
rendered against [`Bindings`](#bindings) at verdict time. No raw strings -
reasons are part of the eDSL so backends can diff / index / route them.

### Bindings

Slot table captured during evaluation, attached to `Deny`/`Escalate`. Lets a
verdict carry the actual values that produced it ("rejected because edge_bps=12

> limit=10").

### Action / Verb

`trait Action: Debug + Clone + Send + Sync + 'static` with `KIND` const +
`Input` / `Output` types +
`lower<B: BackendExt>(&self,
&DagFlow<B>) -> NodeHandle<Input, Output>`. The
open extension point of the eDSL: adopters add new verbs (Buy, Sell, Short,
Transfer, Lend, Stake) by implementing it. Each verb's `lower` splices its
sub-DAG (construct -> risk-check -> sign -> submit -> wait-for-fill) into the
surrounding policy DAG.

### eDSL backends

Eleven independent folds over `RuleNode`: runtime evaluator, markdown digest,
mermaid flowchart, JSON Schema, SMT-LIB export, SQL `WHERE` compiler, proptest
scaffolding, CBOR wire format with schema hash, semantic diff, telemetry
counters, TLA+ predicate export, plus the apalis_workflow `DagFlow` compiler.

## Trade / venue layer

### Ledger

Settlement substrate. `trait Ledger { const NAME; type Address; type
TxId; }`.
Adapters define markers per substrate: `EvmChain { chain_id
}`,
`SvmCluster { ... }`, `BinanceCex`, paper-trading, etc.

### Venue\<L: Ledger\>

A specific tradeable market on a settlement substrate. Carries the identifying
info typed against `L` (a contract address on EVM, a market account on SVM, a
`(symbol, market_kind)` pair on a CEX). Read-side identification only; the
write-side is [`TradingVenue`](#tradingvenuel-ledger).

### TradingVenue\<L: Ledger\>

Write-side extension of [`Venue`](#venuel-ledger): three primitives adopters
implement -

- `check_inventory()` - read balances / positions.
- `place_trade(request)` - submit an order; returns the venue-assigned id once
  accepted.
- `check_order(id)` - query the current state of a previously-placed order.

Implementations almost always perform real I/O and are expected to be called
from inside an apalis Job.

### Feed\<S: Subject\>

Read-only typed event stream from a venue (trades / orders / deposits /
withdrawals). Independent from `TradingVenue` so a single crate can implement
either side - or both - separately.

### OrderRequest / OrderId / Order / OrderState / Inventory

Framework value-object vocabulary in `nuke::order`. Adapters either use them
directly (the EVM venue does) or define their own and map in their
`TradingVenue` impl. `OrderState` enum: `Open`, `PartiallyFilled`, `Filled`,
`Cancelled`, `Rejected`.

## Domain primitives

### Symbol / Side / Px / Qty / Notional

Typed newtypes around `rust_decimal::Decimal` (no `f64` anywhere) in
`nuke::domain`. Typed conversions: `Qty * Px = Notional`, `Notional / Px = Qty`,
`Side::Buy.opposite() = Side::Sell`. The type system rejects "multiply two
prices" / "treat a price as a quantity" mistakes at compile time.

## Support / observability

### Validator / Approved / Refused

Pre-trade gate.
`trait Validator<T> { type Reason; fn check(&self,
&T) -> Result<(), Reason>; }`.
The framework's `validate(&v, input)` free function packages the verdict into
typed `Approved<T>` / `Refused<T, Reason>` wrappers. `Approved::new` is private
to `nuke::review` so a downstream step asking for `Approved<T>` is guaranteed to
have come through some `Validator`.

### Processor\<E\>

Consume events of type `E`. `fn process(&mut self, E) -> Output`. Pipelines
compose `Processor` impls to model work as a chain of typed event handlers.

### Auditor / AuditTick

`AuditTick<Snapshot, Context>` = one audit emission. `Auditor` is the trait
adopters implement to send ticks somewhere (metrics exporter, log file, event
store). Generic over Snapshot / Context so adopters control the typed shape.

### Identifier\<I\>

"I have an id of type `I`." Bare-minimum trait that lots of carrier types
implement so id-routing code can be `impl Identifier<I>`-bound without caring
about the carrier.

### Tx\<Item\>

Generic transmitter. `fn send(&self, Item) -> Result<(), Error>`. Decouples a
producer from its concrete channel type so adopters can swap mpsc / broadcast /
file sink / metrics emitter without touching the producer. `DropTx` ships as a
no-op default.

### Lifecycle hooks

Three independent traits for resilience events the framework can't make a
default decision about:

- `OnDisconnect` returns a `DisconnectAction` (`Reconnect { backoff }` /
  `Shutdown` / `Pause`). Default impl: 1s..30s exponential backoff.
- `OnTradingDisabled` runs when an operator flips trading off.
- `OnShutdown` runs when the run loop is asked to stop.

Each has a `Default*` impl for the no-effort case.

### Stream extensions

`PolicyStreamExt`: `with_index`, `with_timestamp`, `forward_clone_by` (send each
item through a `Tx` before re-yielding it). Pipeline authors use these to fan a
stream out to audit / metrics without breaking the main consumer.
