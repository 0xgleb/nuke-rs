# evm

EVM adapter for the nuke framework: WebSocket Source + (stub) TradingVenue
against any EVM chain.

This crate is **not** part of the framework. It's a worked implementation of
`nuke::Subject` and the framework's `Transport` / `Wire` / `Ledger` / `Venue` /
`TradingVenue` traits against an Ethereum JSON-RPC websocket transport. Examples
and applications opt in by depending on this crate; framework crates (`nuke`,
`nuke-derive`, `event-sorcery`) never do.

## Layout

| Module / type                 | What it provides                                                                                                                                                                                    |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EvmWsSource`                 | Owns one ws connection plus the background task that does JSON-RPC framing and `eth_subscribe` management. Notifications forward as `RawLog`s.                                                      |
| `EvmSubject` (trait + derive) | Extends `nuke::Subject` with the EVM-specific bits: a contract `address()`, `subscription()` (params for `eth_subscribe("logs", ...)`), and `decode(...)`.                                          |
| `Dispatcher<L>`               | Address-keyed decoder map. Populated by the framework's `nuke::Subscribe` walker via this crate's `Wire<H: EvmSubject>` impl on `EvmWsSource`.                                                      |
| `EvmChain`                    | `Ledger` impl. Per-chain marker (chain id distinguishes mainnet / Arbitrum / Base / etc.).                                                                                                          |
| `EvmVenue` / `EvmRpcVenue`    | `Venue<EvmChain>` / `TradingVenue<EvmChain>` impls. The trade-submission path is a v0 stub returning `EvmVenueError::Unwired`; adopters wire a signer.                                              |
| `pump`                        | Wires `EvmWsSource -> nuke::Subscribe -> Dispatcher` and feeds the resulting typed event stream into `nuke::pump_through_apalis`. The framework owns the run loop; this is the EVM-specific bridge. |

## Status

Pre-alpha. The ws transport works and is exercised by the dex_arb example's e2e
test against an embedded mock JSON-RPC server. The write-side `TradingVenue`
impl is a stub - submission, signing, and fill-confirmation wire-in is the next
adapter milestone.
