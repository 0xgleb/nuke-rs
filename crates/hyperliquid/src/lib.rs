//! Hyperliquid Source / TradingVenue adapter for the `nuke` framework.
//!
//! This crate is **not** part of the framework - it's an adapter that
//! wraps `hyperliquid_rust_sdk` behind nuke's `Subject` / `Venue` /
//! `TradingVenue` traits. Examples and applications opt in by depending
//! on this crate; framework crates (`nuke`, `nuke-derive`) never do.
//!
//! Status: stub. The Source and TradingVenue impls have not been
//! written yet; this file currently exists so the crate has a library
//! root and the workspace builds.
