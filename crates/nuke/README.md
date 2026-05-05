# nuke

The framework crate. Public vocabulary at a glance:

| Module       | What it exports                                                                                                                                                                  |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apalis`     | `pump_through_apalis` - the venue-agnostic run loop adapter crates feed.                                                                                                         |
| `auditor`    | `Auditor` + `AuditTick<Snapshot, Context>` for first-class audit emission.                                                                                                       |
| `domain`     | Decimal-based newtypes (`Symbol` / `Side` / `Px` / `Qty` / `Notional`) with typed conversions. Never `f64`.                                                                      |
| `error`      | `Error` / `Result` aliases.                                                                                                                                                      |
| `ext`        | Tower-shaped adapter primitives: `ExtStream`, `ExtQuery`, `Polling` (auto-impls `ExtStream` from any `ExtQuery + tick stream`).                                                  |
| `feed`       | `Feed<S: Subject>` - read-only typed event stream from a venue.                                                                                                                  |
| `identifier` | `Identifier<I>` - "I have a typed id of type `I`".                                                                                                                               |
| `job`        | `Job<Ctx>` - the unit of durable retryable work + the `work` apalis handler with `backon` retries.                                                                               |
| `ledger`     | `Ledger` - settlement substrate abstraction.                                                                                                                                     |
| `lifecycle`  | `OnDisconnect` / `OnTradingDisabled` / `OnShutdown` resilience hooks + their `Default*` impls.                                                                                   |
| `order`      | `OrderRequest` / `OrderId` / `Order` / `OrderState` / `Inventory` value-object vocabulary.                                                                                       |
| `policy`     | The eDSL: typed `Expr<T>` AST, `RuleNode<A>` rule tree, `Action` trait for verbs, eleven backend folds (markdown / mermaid / SMT / SQL / etc.).                                  |
| `pump`       | `inject_ext_stream` lifts an `ExtStream` into the reactor's typed dep union; `pump_dep_streams` fans many lifted streams into one merged event source for `pump_through_apalis`. |
| `reactor`    | `Reactor` trait - typed `react(event) -> Vec<Self::Job>`.                                                                                                                        |
| `review`     | `Validator<T>` + `validate(...)` + typed `Approved<T>` / `Refused<T, Reason>` wrappers.                                                                                          |
| `stream_ext` | `with_index` / `with_timestamp` / `forward_clone_by` stream combinators.                                                                                                         |
| `subject`    | `Subject` - typed marker for one external-stream dep. Refines `event_sorcery::Dep`.                                                                                              |
| `subscribe`  | Generic walker (`Transport` + `Wire<D>` + `Subscribe<L, T>`) over any `event_sorcery::DepList`.                                                                                  |
| `tx`         | `Tx<Item>` generic transmitter + `DropTx` no-op default.                                                                                                                         |
| `venue`      | `Venue<L: Ledger>` + write-side `TradingVenue<L>` (`check_inventory`, `place_trade`, `check_order`).                                                                             |

The type-level dep-list machinery (`Cons` / `Nil` / `Never` / `OneOf` / `Fold` /
`Dependent` / `DepList` / `HasDep` + the `deps!` macro) is re-exported from
`event_sorcery` so the same idiom covers external streams (a `Subject`) and
internal aggregates (an `EventSourced`).

See the workspace [`docs/glossary.md`](../../docs/glossary.md) for definitions
of every term and where it sits in the run loop.

## Hard invariants

Documented in the workspace [`CLAUDE.md`](../../CLAUDE.md) and enforced at
compile time / by clippy / by CI:

- No `f64` in domain code.
- No closures in the eDSL.
- No venue specifics in framework crates (this crate carries no EVM / SVM / CEX
  types - all that lives in adapter crates).
- External-service side-effects only inside `Job::perform`.
- Apalis is internal; the public API stays in nuke vocab.
- ASCII only.

## Status

Pre-alpha. APIs may change. The eDSL backends are stable; the policy ->
apalis_workflow `DagFlow` compiler is v0 (lowers Do leaves; per-node
verdict-layer decomposition is the next iteration).
