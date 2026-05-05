//! Cross-DEX arbitrage example: WETH/USDC on Uniswap V2 vs SushiSwap V2.
//!
//! Subscribes to `Sync(uint112,uint112)` events from both pools,
//! maintains reserves per pool, and fires an `Opportunity` whenever
//! the cross-pool marginal price diverges by more than
//! `threshold_bps`. The "executor" is a no-op that just logs - real
//! trade execution requires a signer and lands when the policy ->
//! Job DAG compiler does.
//!
//! Run live (needs `ETH_WS_RPC_URL` configured via secretspec):
//!
//! ```sh
//! cargo run -p dex-arb
//! ```
//!
//! Run the e2e test against an embedded mock JSON-RPC ws server (no
//! network, no secrets):
//!
//! ```sh
//! cargo test -p dex-arb
//! ```

use std::sync::Arc;

use alloy_sol_types::sol;
use evm::{EvmSubject, EvmWsSource};
use nuke::prelude::*;
use nuke::{Job, Label};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

secretspec_derive::declare_secrets!("../../secretspec.toml");

sol! {
    #[derive(Debug)]
    contract UniswapV2Pair {
        event Sync(uint112 reserve0, uint112 reserve1);
    }
}

#[derive(EvmSubject)]
#[nuke(
    event = UniswapV2Pair::Sync,
    address = "B4e16d0168e52d35CaCD2c6185b44281Ec28C9Dc"
)]
pub struct UniV2WethUsdc;

#[derive(EvmSubject)]
#[nuke(
    event = UniswapV2Pair::Sync,
    address = "397FF1542f962076d0BFE58eA045FfA2d347ACa0"
)]
pub struct SushiV2WethUsdc;

subjects!(ArbBot, [UniV2WethUsdc, SushiV2WethUsdc]);

/// State shared across the two pool handlers - the latest `(reserve0,
/// reserve1)` per pool. `None` means we haven't seen a `Sync` yet.
#[derive(Default, Debug, Clone, Copy)]
struct PoolState {
    reserve0: u128,
    reserve1: u128,
}

impl PoolState {
    /// Marginal price of token1 priced in token0, expressed as the
    /// ratio `reserve0 / reserve1` (Uniswap V2 spot price). Returns
    /// `None` before any `Sync` lands or if `reserve1 == 0`. Uses
    /// `rust_decimal::Decimal` (never `f64`) for all financial math.
    fn price_token1_in_token0(&self) -> Option<Decimal> {
        (self.reserve1 != 0).then(|| Decimal::from(self.reserve0) / Decimal::from(self.reserve1))
    }
}

/// A detected cross-pool arbitrage opportunity. The reactor emits an
/// [`ArbJob::Log`] per opportunity; apalis runs the Job's `perform`
/// (with retries) - in this example that just logs. Real trade
/// execution would be a different `ArbJob` variant whose `perform`
/// calls a `TradingVenue::place_trade`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opportunity {
    pub buy: Pool,
    pub sell: Pool,
    pub edge_bps: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pool {
    UniV2,
    Sushi,
}

/// Shared context passed to every Job's `perform`. In a real bot
/// this carries `TradingVenue` impls, signers, persistence handles,
/// etc.; the example's executor has nothing to inject.
#[derive(Default, Debug)]
pub struct ArbCtx;

/// The job(s) the reactor enqueues. One variant per kind of work the
/// reactor wants apalis to perform on its behalf.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArbJob {
    Log(Opportunity),
}

#[derive(Debug, thiserror::Error)]
pub enum ArbJobError {}

impl Job<ArbCtx> for ArbJob {
    type Error = ArbJobError;

    fn label(&self) -> Label {
        match self {
            ArbJob::Log(_) => Label::new("arb.log_opportunity"),
        }
    }

    async fn perform(&self, _ctx: &ArbCtx) -> Result<(), Self::Error> {
        match self {
            ArbJob::Log(opp) => {
                ::tracing::info!(?opp, "arbitrage opportunity");
                Ok(())
            }
        }
    }
}

/// The reactor. Holds the per-pool reserve state under a single
/// mutex (writes are infrequent - no contention concern at this
/// scale). `react` is a pure decider: it updates state and returns
/// any jobs the framework should enqueue.
pub struct ArbBot {
    threshold_bps: i64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    univ2: PoolState,
    sushi: PoolState,
}

impl ArbBot {
    pub fn new(threshold_bps: i64) -> Self {
        Self {
            threshold_bps,
            state: Mutex::new(State::default()),
        }
    }

    async fn on_univ2(&self, _id: UniV2WethUsdcId, sync: UniswapV2Pair::Sync) -> Vec<ArbJob> {
        let mut state = self.state.lock().await;
        state.univ2 = sync_to_state(&sync);
        check(
            self.threshold_bps,
            state.univ2,
            state.sushi,
            Pool::UniV2,
            Pool::Sushi,
        )
        .into_iter()
        .map(ArbJob::Log)
        .collect()
    }

    async fn on_sushi(&self, _id: SushiV2WethUsdcId, sync: UniswapV2Pair::Sync) -> Vec<ArbJob> {
        let mut state = self.state.lock().await;
        state.sushi = sync_to_state(&sync);
        check(
            self.threshold_bps,
            state.sushi,
            state.univ2,
            Pool::Sushi,
            Pool::UniV2,
        )
        .into_iter()
        .map(ArbJob::Log)
        .collect()
    }
}

#[async_trait]
impl Reactor for ArbBot {
    type Job = ArbJob;
    type Ctx = ArbCtx;

    async fn react(&self, event: <Self::Subjects as SubjectList>::Event) -> Vec<Self::Job> {
        event
            .on(|id, sync| async move { self.on_univ2(id, sync).await })
            .on(|id, sync| async move { self.on_sushi(id, sync).await })
            .exhaustive()
            .await
    }
}

fn sync_to_state(sync: &UniswapV2Pair::Sync) -> PoolState {
    PoolState {
        reserve0: sync.reserve0.to::<u128>(),
        reserve1: sync.reserve1.to::<u128>(),
    }
}

/// Returns `Some(opportunity)` if the price gap (in basis points)
/// exceeds the configured threshold. The "buy" side is the cheaper
/// venue.
fn check(
    threshold_bps: i64,
    just_updated: PoolState,
    other: PoolState,
    just_updated_id: Pool,
    other_id: Pool,
) -> Option<Opportunity> {
    let updated_price = just_updated.price_token1_in_token0()?;
    let other_price = other.price_token1_in_token0()?;
    let edge = ((updated_price / other_price) - Decimal::ONE) * Decimal::from(10_000);
    let edge_bps = edge.round().to_i64().unwrap_or(i64::MAX);
    (edge_bps.abs() > threshold_bps).then(|| {
        let (buy, sell) = if updated_price < other_price {
            (just_updated_id, other_id)
        } else {
            (other_id, just_updated_id)
        };
        Opportunity {
            buy,
            sell,
            edge_bps: edge_bps.abs(),
        }
    })
}

#[tokio::main]
async fn main() -> nuke::Result<()> {
    nuke::tracing::init();

    let secrets = SecretSpec::builder()
        .load()
        .map_err(|error| nuke::Error::msg(format!("secretspec load failed: {error}")))?;
    let url = secrets
        .secrets
        .eth_ws_rpc_url
        .ok_or_else(|| nuke::Error::msg("ETH_WS_RPC_URL not set"))?;

    let chain = EvmWsSource::connect(&url).await?;
    let bot = Arc::new(ArbBot::new(20));
    let ctx = Arc::new(ArbCtx);

    evm::pump(chain, bot, ctx).await
}

// ---------------------------------------------------------------------
// E2e test - examples ARE e2e tests (see CLAUDE.md). Inline
// `#[cfg(test)]` rather than a separate `tests/` directory.
// ---------------------------------------------------------------------

#[cfg(test)]
mod e2e {
    use super::*;
    use std::time::Duration;

    use alloy_primitives::{U256, address};
    use alloy_sol_types::SolEvent;
    use futures_util::StreamExt;
    use tokio::sync::mpsc;

    use mock_chain::{MockEthWsServer, MockLog, ScenarioPlan};

    sol! {
        #[derive(Debug)]
        contract V2 {
            event Sync(uint112 reserve0, uint112 reserve1);
        }
    }

    #[derive(EvmSubject)]
    #[nuke(
        event = V2::Sync,
        address = "1111111111111111111111111111111111111111"
    )]
    struct PoolA;

    #[derive(EvmSubject)]
    #[nuke(
        event = V2::Sync,
        address = "2222222222222222222222222222222222222222"
    )]
    struct PoolB;

    subjects!(TestBot, [PoolA, PoolB]);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    enum Side {
        A,
        B,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    struct TestOpportunity {
        buy: Side,
        sell: Side,
    }

    #[derive(Default, Debug, Clone, Copy)]
    struct Reserves {
        r0: u128,
        r1: u128,
    }

    impl Reserves {
        fn price(&self) -> Option<Decimal> {
            (self.r1 != 0).then(|| Decimal::from(self.r0) / Decimal::from(self.r1))
        }
    }

    #[derive(Default)]
    struct TestState {
        a: Reserves,
        b: Reserves,
    }

    /// Test ctx: a channel the assertion-side drains. The Job's
    /// `perform` writes to it. Demonstrates how a reactor's
    /// per-deployment dependencies (here: a test sink; in production:
    /// venue handles, signers, persistence) reach Jobs through the
    /// framework-injected context rather than reactor closure capture.
    struct TestCtx {
        out: mpsc::Sender<TestOpportunity>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum TestJob {
        Emit(TestOpportunity),
    }

    #[derive(Debug, thiserror::Error)]
    enum TestJobError {}

    impl Job<TestCtx> for TestJob {
        type Error = TestJobError;

        fn label(&self) -> Label {
            Label::new("test.emit_opportunity")
        }

        async fn perform(&self, ctx: &TestCtx) -> Result<(), Self::Error> {
            match self {
                TestJob::Emit(opp) => {
                    let _ = ctx.out.send(*opp).await;
                    Ok(())
                }
            }
        }
    }

    struct TestBot {
        threshold_bps: i64,
        state: Mutex<TestState>,
    }

    impl TestBot {
        fn new(threshold_bps: i64) -> Self {
            Self {
                threshold_bps,
                state: Mutex::new(TestState::default()),
            }
        }

        async fn on_a(&self, _id: PoolAId, sync: V2::Sync) -> Vec<TestJob> {
            let mut state = self.state.lock().await;
            state.a = Reserves {
                r0: sync.reserve0.to::<u128>(),
                r1: sync.reserve1.to::<u128>(),
            };
            test_check(self.threshold_bps, state.a, state.b, Side::A, Side::B)
                .into_iter()
                .map(TestJob::Emit)
                .collect()
        }

        async fn on_b(&self, _id: PoolBId, sync: V2::Sync) -> Vec<TestJob> {
            let mut state = self.state.lock().await;
            state.b = Reserves {
                r0: sync.reserve0.to::<u128>(),
                r1: sync.reserve1.to::<u128>(),
            };
            test_check(self.threshold_bps, state.b, state.a, Side::B, Side::A)
                .into_iter()
                .map(TestJob::Emit)
                .collect()
        }
    }

    #[async_trait]
    impl Reactor for TestBot {
        type Job = TestJob;
        type Ctx = TestCtx;

        async fn react(&self, event: <Self::Subjects as SubjectList>::Event) -> Vec<Self::Job> {
            event
                .on(|id, sync| async move { self.on_a(id, sync).await })
                .on(|id, sync| async move { self.on_b(id, sync).await })
                .exhaustive()
                .await
        }
    }

    fn test_check(
        threshold_bps: i64,
        just_updated: Reserves,
        other: Reserves,
        just_updated_id: Side,
        other_id: Side,
    ) -> Option<TestOpportunity> {
        let updated_price = just_updated.price()?;
        let other_price = other.price()?;
        let edge = ((updated_price / other_price) - Decimal::ONE).abs() * Decimal::from(10_000);
        (edge.round().to_i64().unwrap_or(0) > threshold_bps).then(|| {
            let (buy, sell) = if updated_price < other_price {
                (just_updated_id, other_id)
            } else {
                (other_id, just_updated_id)
            };
            TestOpportunity { buy, sell }
        })
    }

    fn sync_log(addr: alloy_primitives::Address, r0: u128, r1: u128) -> MockLog {
        let event = V2::Sync {
            reserve0: U256::from(r0).to(),
            reserve1: U256::from(r1).to(),
        };
        let log_data = event.encode_log_data();
        MockLog {
            address: addr,
            topics: log_data.topics().to_vec(),
            data: log_data.data,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn detects_arb_opportunities_against_mock_chain() {
        let pool_a = address!("1111111111111111111111111111111111111111");
        let pool_b = address!("2222222222222222222222222222222222222222");

        // Scenario:
        //   1. PoolA Sync(1000, 1000) - first event, other empty -> no opp
        //   2. PoolB Sync(1000, 1000) - both at price 1.0, gap 0   -> no opp
        //   3. PoolA Sync(1500, 1000) - A=1.5, B=1.0, gap 5000 bps -> opp
        //   4. PoolB Sync(1010, 1000) - A=1.5, B=1.01, gap ~4851   -> opp
        //   5. PoolA Sync(1500, 1000) - unchanged, still huge gap  -> opp
        let plan = ScenarioPlan {
            expected_subscriptions: 2,
            events: vec![
                sync_log(pool_a, 1_000, 1_000),
                sync_log(pool_b, 1_000, 1_000),
                sync_log(pool_a, 1_500, 1_000),
                sync_log(pool_b, 1_010, 1_000),
                sync_log(pool_a, 1_500, 1_000),
            ],
            pacing: Duration::from_millis(5),
        };

        let mock = MockEthWsServer::start(plan).await;
        let chain = EvmWsSource::connect(&mock.url)
            .await
            .expect("connect to mock");

        let (out_tx, out_rx) = mpsc::channel(16);
        let bot = Arc::new(TestBot::new(20));
        let ctx = Arc::new(TestCtx { out: out_tx });
        let runner = tokio::spawn(evm::pump(chain, bot, ctx));

        let expected = 3;
        let receiver = Arc::new(tokio::sync::Mutex::new(out_rx));

        let received: Vec<_> = futures_util::stream::iter(0..expected)
            .then(|_| {
                let receiver = Arc::clone(&receiver);
                async move {
                    let mut rx = receiver.lock().await;
                    tokio::time::timeout(Duration::from_secs(5), rx.recv())
                        .await
                        .expect("opportunity arrives within timeout")
                        .expect("channel closed before opportunities arrived")
                }
            })
            .collect()
            .await;

        let expected_opp = TestOpportunity {
            buy: Side::B,
            sell: Side::A,
        };
        assert_eq!(received, vec![expected_opp; expected]);

        // Confirm no extra opportunities sneak through. Both `Err(_)`
        // (timeout before another event) and `Ok(None)` (channel closed
        // because the source finished cleanly) are acceptable terminal
        // states.
        let mut rx = receiver.lock().await;
        let extra = tokio::time::timeout(Duration::from_millis(150), rx.recv()).await;
        match extra {
            Err(_) | Ok(None) => {}
            Ok(Some(opp)) => panic!("unexpected extra opportunity: {opp:?}"),
        }

        runner.abort();
    }
}

// ---------------------------------------------------------------------
// In-process mock JSON-RPC ws server for the e2e test. Speaks just
// enough of the protocol `EvmWsSource` expects: accepts `eth_subscribe`
// with method `logs`, responds with a fresh subscription id per
// address, and replays a pre-baked sequence of `eth_subscription`
// notifications once every expected subscription is confirmed.
// ---------------------------------------------------------------------

#[cfg(test)]
mod mock_chain {
    use std::collections::HashMap;
    use std::time::Duration;

    use alloy_primitives::{Address, B256, Bytes};
    use futures_util::{SinkExt, StreamExt};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio_tungstenite::tungstenite::Message;

    /// One mock log to deliver. Built from an alloy-encoded event.
    pub struct MockLog {
        pub address: Address,
        pub topics: Vec<B256>,
        pub data: Bytes,
    }

    /// Plan handed to the mock at startup.
    pub struct ScenarioPlan {
        pub expected_subscriptions: usize,
        pub events: Vec<MockLog>,
        pub pacing: Duration,
    }

    pub struct MockEthWsServer {
        pub url: String,
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
}
