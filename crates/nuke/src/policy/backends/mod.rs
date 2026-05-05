//! Backends — folds over [`RuleNode`](crate::policy::ast::RuleNode)
//! that produce something other than a [`Decision`](crate::policy::Decision).
//!
//! Each backend is independent and can be enabled/extended without
//! touching the others. New backends slot in here as separate modules.

pub mod json_schema;
pub mod markdown;
pub mod mermaid;
pub mod proptest;
pub mod semantic_diff;
pub mod smt;
pub mod sql;
pub mod telemetry;
pub mod tla;
pub mod wire;
