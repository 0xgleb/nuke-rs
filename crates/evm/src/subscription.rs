//! [`SubscriptionSpec`] (the params for `eth_subscribe`) and [`RawLog`]
//! (the decoded notification payload), plus their JSON-RPC serialization.

use alloy_primitives::{Address, B256, Bytes, LogData};
use serde::{Deserialize, Serialize};

/// Parameters for an `eth_subscribe("logs", ...)` call.
///
/// Built by [`Subject::subscription`](crate::Subject::subscription) -
/// usually through `#[derive(EvmSubject)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionSpec {
    pub address: Address,
    pub topics: Vec<B256>,
}

impl SubscriptionSpec {
    /// Subscribe to a single contract address filtered on a single
    /// `topic0` (event signature hash).
    pub fn logs_for(address: Address, topic0: B256) -> Self {
        Self {
            address,
            topics: vec![topic0],
        }
    }
}

/// One log delivered by an `eth_subscription` notification.
///
/// Holds enough to ABI-decode via the alloy `SolEvent` machinery
/// (which works off `LogData`). Block / transaction metadata is
/// intentionally omitted from v0; revisit once a use case needs it.
#[derive(Debug, Clone)]
pub struct RawLog {
    pub address: Address,
    pub log_data: LogData,
}

impl RawLog {
    /// Build from raw eth_subscribe fields.
    pub fn new(address: Address, topics: Vec<B256>, data: Bytes) -> Self {
        Self {
            address,
            log_data: LogData::new_unchecked(topics, data),
        }
    }

    /// Borrow the alloy [`LogData`] for ABI decoding.
    pub fn data(&self) -> &LogData {
        &self.log_data
    }
}
