//! Cross-DEX arbitrage example: WETH/USDC on Uniswap V2 vs SushiSwap V2.
//!
//! Subscribes to `Sync(uint112,uint112)` events from both pools, maintains
//! reserves per pool, and fires an `Opportunity` whenever the cross-pool
//! marginal price diverges by more than `threshold_bps`. The "executor" is
//! a no-op that just logs — real trade execution requires a signer and
//! comes in a follow-up.
//!
//! ```sh
//! ETH_WS_RPC_URL=wss://... cargo run --example arb_bot
//! ```

use std::sync::Arc;

use alloy_sol_types::sol;
use nuke::evm::EvmWsSource;
use nuke::prelude::*;
use tokio::sync::Mutex;

secretspec_derive::declare_secrets!("secretspec.toml");

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

/// State shared across the two pool handlers — the latest `(reserve0,
/// reserve1)` per pool. `None` means we haven't seen a `Sync` yet.
#[derive(Default, Debug, Clone, Copy)]
struct PoolState {
    reserve0: u128,
    reserve1: u128,
}

impl PoolState {
    /// Marginal price of token1 priced in token0, expressed as the ratio
    /// `reserve0 / reserve1` (Uniswap V2 spot price). Returns `None`
    /// before any `Sync` lands or if `reserve1 == 0`.
    fn price_token1_in_token0(&self) -> Option<f64> {
        if self.reserve1 == 0 {
            None
        } else {
            Some(self.reserve0 as f64 / self.reserve1 as f64)
        }
    }
}

/// A detected cross-pool arbitrage opportunity. The "executor" worker
/// receives one of these per fire.
#[derive(Debug, Clone, Copy)]
pub struct Opportunity {
    pub buy: Pool,
    pub sell: Pool,
    pub edge_bps: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    UniV2,
    Sushi,
}

#[derive(Debug, thiserror::Error)]
pub enum ArbError {}

/// The reactor. Holds the per-pool reserve state under a single mutex
/// (writes are infrequent — no contention concern at this scale).
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

    async fn on_univ2(
        &self,
        _id: UniV2WethUsdcId,
        sync: UniswapV2Pair::Sync,
    ) -> Result<(), ArbError> {
        let mut state = self.state.lock().await;
        state.univ2 = sync_to_state(&sync);
        if let Some(opp) = check(
            self.threshold_bps,
            state.univ2,
            state.sushi,
            Pool::UniV2,
            Pool::Sushi,
        ) {
            self.emit(opp);
        }
        Ok(())
    }

    async fn on_sushi(
        &self,
        _id: SushiV2WethUsdcId,
        sync: UniswapV2Pair::Sync,
    ) -> Result<(), ArbError> {
        let mut state = self.state.lock().await;
        state.sushi = sync_to_state(&sync);
        if let Some(opp) = check(
            self.threshold_bps,
            state.sushi,
            state.univ2,
            Pool::Sushi,
            Pool::UniV2,
        ) {
            self.emit(opp);
        }
        Ok(())
    }

    fn emit(&self, opp: Opportunity) {
        ::tracing::info!(?opp, "arbitrage opportunity");
    }
}

#[async_trait]
impl Reactor for ArbBot {
    type Error = ArbError;

    async fn react(
        &self,
        event: <Self::Subjects as SubjectList>::Event,
    ) -> Result<(), Self::Error> {
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

/// Check the just-updated pool against the other one. Returns
/// `Some(opportunity)` if the price gap (in basis points) exceeds the
/// configured threshold. The "buy" side is the cheaper venue.
fn check(
    threshold_bps: i64,
    just_updated: PoolState,
    other: PoolState,
    just_updated_id: Pool,
    other_id: Pool,
) -> Option<Opportunity> {
    let updated_price = just_updated.price_token1_in_token0()?;
    let other_price = other.price_token1_in_token0()?;
    let edge = ((updated_price / other_price) - 1.0) * 10_000.0;
    let edge_bps = edge.round() as i64;
    if edge_bps.abs() <= threshold_bps {
        return None;
    }
    let (buy, sell) = if updated_price < other_price {
        (just_updated_id, other_id)
    } else {
        (other_id, just_updated_id)
    };
    Some(Opportunity {
        buy,
        sell,
        edge_bps: edge_bps.abs(),
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
    let bot = Arc::new(ArbBot::new(/* threshold_bps */ 20));

    nuke::run(chain, bot).await
}
