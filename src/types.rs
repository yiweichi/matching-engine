pub type OrderId = u64;
pub type Price = u64;
pub type Qty = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderType {
    /// Rests on the book if not fully filled.
    Limit,
    /// Fills what it can, remainder is cancelled.
    Market,
}

#[derive(Debug, Clone, Copy)]
pub struct Order {
    pub id: OrderId,
    pub side: Side,
    /// Price in ticks. Ignored for Market orders.
    pub price: Price,
    pub qty: Qty,
    pub order_type: OrderType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    pub maker_id: OrderId,
    pub taker_id: OrderId,
    pub price: Price,
    pub qty: Qty,
    pub side: Side,
}
