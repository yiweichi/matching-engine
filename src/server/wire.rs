#![allow(dead_code)]

// ── Wire protocol constants (must match C++ include/nts/wire_protocol.h) ─────

pub const MD_MSG_QUOTE: u8 = 1;
pub const MD_MSG_DEPTH: u8 = 2;
pub const MD_MSG_TRADE: u8 = 3;
pub const MD_MSG_REFERENCE: u8 = 4;

pub const ORDER_MSG_NEW: u8 = 1;
pub const ORDER_MSG_CANCEL: u8 = 2;

pub const SIDE_BUY: u8 = 0;
pub const SIDE_SELL: u8 = 1;

pub const ORDER_TYPE_LIMIT: u8 = 0;
pub const ORDER_TYPE_MARKET: u8 = 1;
pub const ORDER_TYPE_IOC_LIMIT: u8 = 2;

pub const EXEC_NEW_ACK: u8 = 0;
pub const EXEC_FILL: u8 = 1;
pub const EXEC_PARTIAL_FILL: u8 = 2;
pub const EXEC_CANCEL_ACK: u8 = 3;
pub const EXEC_REJECT: u8 = 4;
pub const EXEC_CANCEL_REJECT: u8 = 5;

pub const DEFAULT_MD_PORT: u16 = 12345;
pub const DEFAULT_ORDER_PORT: u16 = 12346;
pub const DEFAULT_INSTRUMENT_ID: u32 = 1;

// ── Market data wire structs (byte-identical to C++ nts::MdHeader etc.) ──────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireMdHeader {
    pub timestamp_ns: u64,
    pub instrument_id: u32,
    pub sequence_num: u32,
    pub msg_type: u8,
    pub _pad: [u8; 7],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireMdQuote {
    pub header: WireMdHeader,
    pub bid_price: f64,
    pub ask_price: f64,
    pub bid_size: u32,
    pub ask_size: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireMdReference {
    pub header: WireMdHeader,
    pub reference_mid: f64,
    pub _pad: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireMdDepthLevel {
    pub price: f64,
    pub size: u32,
    pub order_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireMdDepth {
    pub header: WireMdHeader,
    pub bid_levels: u8,
    pub ask_levels: u8,
    pub _pad: [u8; 6],
    pub bids: [WireMdDepthLevel; 10],
    pub asks: [WireMdDepthLevel; 10],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireMdTrade {
    pub header: WireMdHeader,
    pub price: f64,
    pub size: u32,
    pub aggressor_side: u8,
    pub _pad: [u8; 3],
}

// ── Order / execution report wire structs ────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireOrderMsg {
    pub msg_type: u8,
    pub side: u8,
    pub order_type: u8,
    pub _pad1: [u8; 5],
    pub client_order_id: u64,
    pub price: f64,
    pub qty: u32,
    pub _pad2: u32,
    pub cancel_order_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WireExecReport {
    pub exec_type: u8,
    pub side: u8,
    pub _pad1: [u8; 2],
    pub fill_qty: u32,
    pub order_id: u64,
    pub fill_price: f64,
    pub leaves_qty: u32,
    pub _pad2: u32,
    pub timestamp_ns: u64,
}

// ── Compile-time size assertions ─────────────────────────────────────────────

const _: () = {
    assert!(std::mem::size_of::<WireMdHeader>() == 24);
    assert!(std::mem::size_of::<WireMdQuote>() == 48);
    assert!(std::mem::size_of::<WireMdReference>() == 40);
    assert!(std::mem::size_of::<WireMdDepthLevel>() == 16);
    assert!(std::mem::size_of::<WireMdDepth>() == 352);
    assert!(std::mem::size_of::<WireMdTrade>() == 40);
    assert!(std::mem::size_of::<WireOrderMsg>() == 40);
    assert!(std::mem::size_of::<WireExecReport>() == 40);
};

// ── Byte conversion helpers ──────────────────────────────────────────────────

/// # Safety
/// `val` must be a valid `#[repr(C)]` struct with no padding-dependent invariants.
pub unsafe fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    std::slice::from_raw_parts(val as *const T as *const u8, std::mem::size_of::<T>())
}

/// # Safety
/// `bytes` must contain a valid bit pattern for `T` and be at least `size_of::<T>()` bytes.
pub unsafe fn from_bytes<T: Copy>(bytes: &[u8]) -> T {
    debug_assert!(bytes.len() >= std::mem::size_of::<T>());
    std::ptr::read_unaligned(bytes.as_ptr() as *const T)
}

pub fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

// ── Side conversion ──────────────────────────────────────────────────────────

use matching_engine::Side;

pub fn side_to_wire(side: Side) -> u8 {
    match side {
        Side::Buy => SIDE_BUY,
        Side::Sell => SIDE_SELL,
    }
}

pub fn wire_to_side(wire: u8) -> Side {
    if wire == SIDE_BUY {
        Side::Buy
    } else {
        Side::Sell
    }
}
