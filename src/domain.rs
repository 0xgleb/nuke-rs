//! Domain primitives for trading and asset transfer.
//!
//! Newtypes per financial primitive over [`rust_decimal::Decimal`]. **No
//! `f64` anywhere** in domain code — floats lose precision in ways that
//! are catastrophic at financial scale, and the eDSL backends (SMT
//! solvers especially) assume exact arithmetic.
//!
//! - [`Symbol`] — opaque instrument identifier.
//! - [`Side`] — discriminated `Buy | Sell` (boolean blindness avoided).
//! - [`Px`] — a price.
//! - [`Qty`] — a quantity (e.g. a swap size, an inventory level).
//! - [`Notional`] — a money amount.
//!
//! Conversions across primitives are *typed*: `Qty * Px = Notional` is
//! the only multiplication you can write between those two; the result
//! carries the right unit so call sites can't accidentally treat a price
//! as a quantity or vice versa.

use std::fmt;
use std::ops::{Add, Sub};

use rust_decimal::Decimal;

/// Opaque instrument identifier — e.g. `"BTC-USD"`, `"WETH/USDC"`. Wrap
/// once at the boundary; callers never touch the raw string.
///
/// String-backed in v0; switch to interned `&'static str` once the rule
/// registry needs it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol(String);

impl Symbol {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Trade side. Discriminated union, never `bool` — boolean blindness at
/// call sites obscures direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        })
    }
}

/// A price expressed as a quote-currency amount per unit base. Always
/// `Decimal` — never `f64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Px(Decimal);

impl Px {
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    pub const fn into_inner(self) -> Decimal {
        self.0
    }

    pub const fn as_decimal(&self) -> &Decimal {
        &self.0
    }
}

impl fmt::Display for Px {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A quantity — a count of base-currency units (e.g. shares, tokens, lots).
/// Always `Decimal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Qty(Decimal);

impl Qty {
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    pub const fn into_inner(self) -> Decimal {
        self.0
    }

    pub const fn as_decimal(&self) -> &Decimal {
        &self.0
    }
}

impl fmt::Display for Qty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl Add for Qty {
    type Output = Self;
    fn add(self, other: Self) -> Self::Output {
        Self(self.0 + other.0)
    }
}

impl Sub for Qty {
    type Output = Self;
    fn sub(self, other: Self) -> Self::Output {
        Self(self.0 - other.0)
    }
}

/// A money amount — quote-currency denominated value (e.g. `$1234.56` of
/// USDC). Always `Decimal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Notional(Decimal);

impl Notional {
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    pub const fn into_inner(self) -> Decimal {
        self.0
    }

    pub const fn as_decimal(&self) -> &Decimal {
        &self.0
    }
}

impl fmt::Display for Notional {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl Add for Notional {
    type Output = Self;
    fn add(self, other: Self) -> Self::Output {
        Self(self.0 + other.0)
    }
}

impl Sub for Notional {
    type Output = Self;
    fn sub(self, other: Self) -> Self::Output {
        Self(self.0 - other.0)
    }
}

/// `Qty * Px = Notional`. Typed multiplication: callers can't
/// accidentally multiply two prices or two quantities and get a
/// nonsense type.
impl std::ops::Mul<Px> for Qty {
    type Output = Notional;
    fn mul(self, price: Px) -> Notional {
        Notional(self.0 * price.0)
    }
}

impl std::ops::Mul<Qty> for Px {
    type Output = Notional;
    fn mul(self, quantity: Qty) -> Notional {
        Notional(self.0 * quantity.0)
    }
}

/// `Notional / Px = Qty`. Inverse of the multiplication above.
///
/// # Panics
///
/// Panics if `price.into_inner()` is zero. Callers that can't guarantee a
/// non-zero price should check first.
impl std::ops::Div<Px> for Notional {
    type Output = Qty;
    fn div(self, price: Px) -> Qty {
        Qty(self.0 / price.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: i64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn qty_times_px_yields_notional() {
        let qty = Qty::new(d(10));
        let px = Px::new(d(7));
        let notional: Notional = qty * px;
        assert_eq!(notional, Notional::new(d(70)));
    }

    #[test]
    fn px_times_qty_yields_notional_same_result() {
        let qty = Qty::new(d(10));
        let px = Px::new(d(7));
        assert_eq!(qty * px, px * qty);
    }

    #[test]
    fn notional_div_px_recovers_qty() {
        let notional = Notional::new(d(70));
        let px = Px::new(d(7));
        let qty: Qty = notional / px;
        assert_eq!(qty, Qty::new(d(10)));
    }

    #[test]
    fn side_opposite_is_involutive() {
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite().opposite(), Side::Sell);
    }

    #[test]
    fn symbol_round_trips_through_display() {
        let symbol = Symbol::new("WETH/USDC");
        assert_eq!(symbol.as_str(), "WETH/USDC");
        assert_eq!(format!("{symbol}"), "WETH/USDC");
    }
}
