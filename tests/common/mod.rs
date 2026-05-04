//! In-process mock JSON-RPC websocket server for integration tests.
//!
//! Speaks just enough of the protocol nuke's `EvmWsSource` expects:
//! accepts `eth_subscribe("logs", ...)` requests, responds with a fresh
//! subscription id per address, and replays a pre-baked sequence of
//! `eth_subscription` notifications once every expected subscription
//! has been confirmed.

#![allow(dead_code)] // Helper module — items used selectively by individual tests.

use std::collections::HashMap;
use std::time::Duration;

use alloy_primitives::{Address, B256, Bytes};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

/// One mock log to deliver. Built directly from an alloy-encoded event.
pub struct MockLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
}

/// Plan handed to the mock at startup. Once `expected_subscriptions` have
/// been confirmed, the server starts emitting `events` in order, paced
/// by `pacing` between frames.
pub struct ScenarioPlan {
    pub expected_subscriptions: usize,
    pub events: Vec<MockLog>,
    pub pacing: Duration,
}

pub struct MockEthWsServer {
    pub url: String,
    /// Held to keep the session task alive for the lifetime of the
    /// server handle.
    _handle: JoinHandle<()>,
}

impl MockEthWsServer {
    pub async fn start(plan: ScenarioPlan) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind localhost");
        let port = listener.local_addr().expect("local addr").port();
        let url = format!("ws://127.0.0.1:{port}");
        let handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept ws connection");
            let ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("ws handshake");
            run_session(ws, plan).await;
        });
        Self {
            url,
            _handle: handle,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    id: u64,
    #[serde(default)]
    method: String,
    #[serde(default)]
    params: Value,
}

async fn run_session(
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    plan: ScenarioPlan,
) {
    let (mut sink, mut stream) = ws.split();
    let mut subscriptions: HashMap<Address, String> = HashMap::new();

    while subscriptions.len() < plan.expected_subscriptions {
        let frame = stream
            .next()
            .await
            .expect("client should send eth_subscribe before disconnecting")
            .expect("ws read");
        let text = match frame {
            Message::Text(text) => text,
            Message::Ping(payload) => {
                sink.send(Message::Pong(payload)).await.ok();
                continue;
            }
            Message::Close(_) => return,
            _ => continue,
        };
        let request: RpcRequest = serde_json::from_str(&text).expect("valid JSON-RPC");
        assert_eq!(
            request.method, "eth_subscribe",
            "only eth_subscribe is supported"
        );

        let address = extract_logs_address(&request.params);
        let sub_id = format!("0x{:x}", subscriptions.len() + 1);
        subscriptions.insert(address, sub_id.clone());

        let response = json!({
            "jsonrpc": "2.0",
            "id": request.id,
            "result": sub_id,
        });
        sink.send(Message::Text(response.to_string().into()))
            .await
            .expect("send subscription ack");
    }

    for event in plan.events {
        let sub_id = subscriptions
            .get(&event.address)
            .unwrap_or_else(|| panic!("event for unsubscribed address {:?}", event.address));
        let frame = json!({
            "jsonrpc": "2.0",
            "method": "eth_subscription",
            "params": {
                "subscription": sub_id,
                "result": {
                    "address": event.address,
                    "topics": event.topics,
                    "data": event.data,
                },
            },
        });
        sink.send(Message::Text(frame.to_string().into()))
            .await
            .expect("send notification");
        tokio::time::sleep(plan.pacing).await;
    }
}

/// Pull the address out of `["logs", { "address": ..., "topics": ... }]`.
fn extract_logs_address(params: &Value) -> Address {
    let array = params.as_array().expect("params is array");
    assert_eq!(array.first().and_then(Value::as_str), Some("logs"));
    let filter = array
        .get(1)
        .and_then(Value::as_object)
        .expect("logs filter object");
    let address_value = filter.get("address").expect("address in filter");
    serde_json::from_value(address_value.clone()).expect("decode address")
}
