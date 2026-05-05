//! Websocket transport for Ethereum JSON-RPC subscriptions.
//!
//! [`EvmWsSource`] owns the websocket and a background task that does
//! JSON-RPC framing. `subscribe` issues `eth_subscribe`, awaits the
//! returned subscription id, and registers it so that subsequent
//! `eth_subscription` notifications are forwarded as [`RawLog`] values
//! into a single `mpsc` channel consumed by [`pump`](crate::evm::pump).
//!
//! Reconnect, backoff, and heartbeat are intentionally absent in v0 -
//! follow-up PRs will layer them on top of this transport via tower
//! middleware.

use std::collections::HashMap;
use std::sync::Arc;

use alloy_primitives::{Address, B256, Bytes};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::subscription::{RawLog, SubscriptionSpec};
use nuke::{Error, Result};

/// JSON-RPC level error (malformed message, unexpected id, server
/// error). Boxed into `nuke::Error::Transport` at the framework
/// boundary so the framework's error enum stays venue-agnostic.
#[derive(Debug, thiserror::Error)]
#[error("JSON-RPC error: {0}")]
pub struct JsonRpcError(pub String);

/// Single websocket connection to an Ethereum JSON-RPC endpoint.
///
/// Exposes a high-level `subscribe(spec)` for issuing `eth_subscribe`
/// and a one-shot `take_log_stream()` for the consumer (the
/// [`pump`](crate::evm::pump) loop).
pub struct EvmWsSource {
    rpc_tx: mpsc::Sender<RpcCommand>,
    log_rx: Option<mpsc::Receiver<RawLog>>,
}

impl EvmWsSource {
    /// Connect to the given JSON-RPC websocket URL and spawn the
    /// background framing task. Returns once the ws handshake is done.
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws_stream, _response) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| Error::Transport(Box::new(error)))?;

        let (rpc_tx, rpc_rx) = mpsc::channel::<RpcCommand>(64);
        let (log_tx, log_rx) = mpsc::channel::<RawLog>(1024);

        tokio::spawn(framing_task(ws_stream, rpc_rx, log_tx));

        Ok(Self {
            rpc_tx,
            log_rx: Some(log_rx),
        })
    }

    /// Issue `eth_subscribe("logs", spec)` and register the returned
    /// subscription id. Returns once the server has acknowledged the
    /// subscription.
    pub async fn subscribe(&self, spec: SubscriptionSpec) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.rpc_tx
            .send(RpcCommand::Subscribe {
                spec,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Transport("framing task closed".into()))?;
        reply_rx
            .await
            .map_err(|_| Error::Transport("framing task dropped reply".into()))?
    }

    /// Take ownership of the log notification stream. Subsequent calls
    /// return `None`. Used once by [`pump`](crate::evm::pump) at startup.
    pub fn take_log_stream(&mut self) -> Option<mpsc::Receiver<RawLog>> {
        self.log_rx.take()
    }
}

enum RpcCommand {
    Subscribe {
        spec: SubscriptionSpec,
        reply: oneshot::Sender<Result<()>>,
    },
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: &'a Value,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WsFrame {
    Response {
        #[allow(dead_code)]
        jsonrpc: String,
        id: u64,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<JsonRpcErrorPayload>,
    },
    Notification {
        #[allow(dead_code)]
        jsonrpc: String,
        #[allow(dead_code)]
        method: String,
        params: NotificationParams,
    },
}

#[derive(Debug, Deserialize)]
struct JsonRpcErrorPayload {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
struct NotificationParams {
    subscription: String,
    result: WireLog,
}

#[derive(Debug, Deserialize)]
struct WireLog {
    address: Address,
    #[serde(default)]
    topics: Vec<B256>,
    data: Bytes,
}

async fn framing_task(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut rpc_rx: mpsc::Receiver<RpcCommand>,
    log_tx: mpsc::Sender<RawLog>,
) {
    let (mut sink, mut stream) = ws_stream.split();
    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<()>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let subscriptions: Arc<Mutex<HashMap<String, ()>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut next_id: u64 = 1;

    loop {
        tokio::select! {
            command = rpc_rx.recv() => {
                let Some(command) = command else { break };
                match command {
                    RpcCommand::Subscribe { spec, reply } => {
                        let id = next_id;
                        next_id += 1;
                        let params = json!([
                            "logs",
                            {
                                "address": spec.address,
                                "topics": spec.topics,
                            }
                        ]);
                        let request = JsonRpcRequest {
                            jsonrpc: "2.0",
                            id,
                            method: "eth_subscribe",
                            params: &params,
                        };
                        let payload = match serde_json::to_string(&request) {
                            Ok(payload) => payload,
                            Err(error) => {
                                let _ = reply.send(Err(Error::Transport(JsonRpcError(error.to_string()).into())));
                                continue;
                            }
                        };
                        pending.lock().await.insert(id, reply);
                        if let Err(error) = sink.send(Message::Text(payload.into())).await {
                            let reply = pending.lock().await.remove(&id);
                            if let Some(reply) = reply {
                                let _ = reply.send(Err(Error::Transport(Box::new(error))));
                            }
                        }
                    }
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(error) => {
                        tracing::warn!(?error, "ws read error; shutting down");
                        break;
                    }
                };
                let text = match frame {
                    Message::Text(text) => text,
                    Message::Ping(payload) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };
                let parsed: WsFrame = match serde_json::from_str(&text) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        tracing::warn!(?error, %text, "invalid JSON-RPC frame");
                        continue;
                    }
                };
                match parsed {
                    WsFrame::Response { id, result, error, .. } => {
                        let reply = pending.lock().await.remove(&id);
                        if let Some(reply) = reply {
                            let outcome = match (result, error) {
                                (Some(Value::String(sub_id)), None) => {
                                    subscriptions.lock().await.insert(sub_id, ());
                                    Ok(())
                                }
                                (_, Some(error)) => Err(Error::Transport(JsonRpcError(error.message).into())),
                                (other, _) => Err(Error::Transport(
                                    JsonRpcError(format!(
                                        "unexpected eth_subscribe result: {other:?}"
                                    ))
                                    .into(),
                                )),
                            };
                            let _ = reply.send(outcome);
                        }
                    }
                    WsFrame::Notification { params, .. } => {
                        if !subscriptions.lock().await.contains_key(&params.subscription) {
                            tracing::warn!(
                                subscription = %params.subscription,
                                "notification for unknown subscription id"
                            );
                            continue;
                        }
                        let log = RawLog::new(
                            params.result.address,
                            params.result.topics,
                            params.result.data,
                        );
                        if log_tx.send(log).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }
}
