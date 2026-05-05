//! Generic id-getter trait. Borrowed from barter-rs's `Identifier`.
//!
//! Lots of types in a trading system carry a stable typed id but have
//! no other shared interface (orders, positions, instruments, fills,
//! ...). [`Identifier<I>`] is the bare minimum: "I have an id of
//! type `I`". Pipeline code that only needs the id (for routing,
//! correlation, lookup) takes `impl Identifier<I>` and stays
//! decoupled from the carrier type.

/// "I have a typed id of type `I`."
///
/// Use as a generic bound on functions that route / look up / correlate
/// by id without caring what the carrier is.
///
/// ```ignore
/// fn route<E: Identifier<OrderId>>(event: &E) -> Worker { ... }
/// ```
pub trait Identifier<I> {
    /// Borrow this value's id.
    fn id(&self) -> &I;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Order {
        order_id: u64,
    }

    impl Identifier<u64> for Order {
        fn id(&self) -> &u64 {
            &self.order_id
        }
    }

    #[test]
    fn identifier_borrows_typed_id() {
        let order = Order { order_id: 42 };
        let id: &u64 = order.id();
        assert_eq!(*id, 42);
    }
}
