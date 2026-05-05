# evm-derive

Proc-macro for the EVM adapter.

| Macro                   | What it does                                                                                                                                                                                                              |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[derive(EvmSubject)]` | From `#[nuke(event = ABI::Variant, address = "0x...")]`, generates a typed Id newtype, the `nuke::Dep` impl, the `nuke::Subject` impl, and the `evm::EvmSubject` impl (with `address()` / `subscription()` / `decode()`). |

Per-pool / per-contract subjects shrink to a one-line struct + the derive:
`#[derive(EvmSubject)] #[nuke(event = UniswapV2Pair::Sync,
address = "...")] pub struct UniV2WethUsdc;`.
