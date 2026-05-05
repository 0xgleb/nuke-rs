//! End-to-end test: drive the same library API the `arb_bot` example
//! uses against an embedded mock JSON-RPC ws server. Asserts that the
//! reactor fires opportunities deterministically given a fixture of
//! `Sync` events.

mod common;

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{U256, address};
use alloy_sol_types::{SolEvent, sol};
use nuke::evm::EvmWsSource;
use nuke::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tokio::sync::{Mutex, mpsc};

use common::{MockEthWsServer, MockLog, ScenarioPlan};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Opportunity {
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
        if self.r1 == 0 {
            None
        } else {
            Some(Decimal::from(self.r0) / Decimal::from(self.r1))
        }
    }
}

#[derive(Default)]
struct State {
    a: Reserves,
    b: Reserves,
}

struct TestBot {
    threshold_bps: i64,
    state: Mutex<State>,
    out: mpsc::Sender<Opportunity>,
}

#[derive(Debug, thiserror::Error)]
enum TestBotError {}

impl TestBot {
    fn new(threshold_bps: i64, out: mpsc::Sender<Opportunity>) -> Self {
        Self {
            threshold_bps,
            state: Mutex::new(State::default()),
            out,
        }
    }

    async fn on_a(&self, _id: PoolAId, sync: V2::Sync) -> Result<(), TestBotError> {
        let mut state = self.state.lock().await;
        state.a = Reserves {
            r0: sync.reserve0.to::<u128>(),
            r1: sync.reserve1.to::<u128>(),
        };
        if let Some(opp) = check(self.threshold_bps, state.a, state.b, Side::A, Side::B) {
            let _ = self.out.send(opp).await;
        }
        Ok(())
    }

    async fn on_b(&self, _id: PoolBId, sync: V2::Sync) -> Result<(), TestBotError> {
        let mut state = self.state.lock().await;
        state.b = Reserves {
            r0: sync.reserve0.to::<u128>(),
            r1: sync.reserve1.to::<u128>(),
        };
        if let Some(opp) = check(self.threshold_bps, state.b, state.a, Side::B, Side::A) {
            let _ = self.out.send(opp).await;
        }
        Ok(())
    }
}

#[async_trait]
impl Reactor for TestBot {
    type Error = TestBotError;

    async fn react(
        &self,
        event: <Self::Subjects as SubjectList>::Event,
    ) -> Result<(), Self::Error> {
        event
            .on(|id, sync| async move { self.on_a(id, sync).await })
            .on(|id, sync| async move { self.on_b(id, sync).await })
            .exhaustive()
            .await
    }
}

fn check(
    threshold_bps: i64,
    just_updated: Reserves,
    other: Reserves,
    just_updated_id: Side,
    other_id: Side,
) -> Option<Opportunity> {
    let updated_price = just_updated.price()?;
    let other_price = other.price()?;
    let edge = ((updated_price / other_price) - Decimal::ONE).abs() * Decimal::from(10_000);
    if edge.round().to_i64().unwrap_or(0) <= threshold_bps {
        return None;
    }
    let (buy, sell) = if updated_price < other_price {
        (just_updated_id, other_id)
    } else {
        (other_id, just_updated_id)
    };
    Some(Opportunity { buy, sell })
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
    //   1. PoolA Sync(1000, 1000)            — first event, other empty → no opp
    //   2. PoolB Sync(1000, 1000)            — both at price 1.0, gap 0  → no opp
    //   3. PoolA Sync(1500, 1000)            — A=1.5, B=1.0, gap 5000 bps → opp
    //   4. PoolB Sync(1010, 1000)            — A=1.5, B=1.01, gap ~4851 bps → opp
    //   5. PoolA Sync(1500, 1000)            — unchanged price, still huge gap → opp
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

    let (out_tx, mut out_rx) = mpsc::channel(16);
    let bot = Arc::new(TestBot::new(20, out_tx));
    let runner = tokio::spawn(nuke::run(chain, bot));

    let expected = 3;
    let mut received = Vec::with_capacity(expected);
    for _ in 0..expected {
        let opp = tokio::time::timeout(Duration::from_secs(5), out_rx.recv())
            .await
            .expect("opportunity arrives within timeout")
            .expect("channel closed before opportunities arrived");
        received.push(opp);
    }

    assert_eq!(received.len(), expected);
    assert_eq!(
        received[0],
        Opportunity {
            buy: Side::B,
            sell: Side::A
        }
    );
    assert_eq!(
        received[1],
        Opportunity {
            buy: Side::B,
            sell: Side::A
        }
    );
    assert_eq!(
        received[2],
        Opportunity {
            buy: Side::B,
            sell: Side::A
        }
    );

    // Confirm no extra opportunities sneak through. Both `Err(_)` (timeout
    // before another event) and `Ok(None)` (channel closed because the
    // source finished cleanly) are acceptable terminal states.
    let extra = tokio::time::timeout(Duration::from_millis(150), out_rx.recv()).await;
    match extra {
        Err(_) | Ok(None) => {}
        Ok(Some(opp)) => panic!("unexpected extra opportunity: {opp:?}"),
    }

    runner.abort();
}
