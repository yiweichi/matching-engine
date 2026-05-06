use std::io::{self, Read, Write};
use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use matching_engine::{Fill, Qty, Side};
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use super::wire;
use crate::arg::ServeArgs;
use crate::sim::exchange::{SimExchange, SimExchangeConfig, StaleOutcome, L1};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
const TIE_BREAKER_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

extern "C" fn handle_shutdown_signal(_signal: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

fn install_shutdown_handlers() {
    SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_shutdown_signal as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = 0;
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
    }
}

struct Client {
    id: u64,
    stream: TcpStream,
    read_buf: Vec<u8>,
    position: i64,
    cash: i64,
    liquidation_pnl: f64,
    orders_total: u64,
    orders_accepted: u64,
    orders_filled: u64,
    orders_rejected: u64,
    fills: u64,
    buys: u64,
    sells: u64,
    buy_qty: u64,
    sell_qty: u64,
    liquidation_fills: u64,
    liquidation_buys: u64,
    liquidation_sells: u64,
    liquidation_buy_qty: u64,
    liquidation_sell_qty: u64,
    liquidation_buy_notional: u64,
    liquidation_sell_notional: u64,
    liquidation_cash: i64,
    pre_liq_position: i64,
    missed_ioc: u64,
    missed_ioc_buys: u64,
    missed_ioc_sells: u64,
    missed_ioc_buy_gap_sum: u64,
    missed_ioc_sell_gap_sum: u64,
    stale_fills: u64,
    stale_misses: u64,
    stale_arrival_lag_ticks_sum: u64,
    disconnected: bool,
}

impl Client {
    fn new(id: u64, stream: TcpStream) -> Self {
        Self {
            id,
            stream,
            read_buf: Vec::with_capacity(4096),
            position: 0,
            cash: 0,
            liquidation_pnl: 0.0,
            orders_total: 0,
            orders_accepted: 0,
            orders_filled: 0,
            orders_rejected: 0,
            fills: 0,
            buys: 0,
            sells: 0,
            buy_qty: 0,
            sell_qty: 0,
            liquidation_fills: 0,
            liquidation_buys: 0,
            liquidation_sells: 0,
            liquidation_buy_qty: 0,
            liquidation_sell_qty: 0,
            liquidation_buy_notional: 0,
            liquidation_sell_notional: 0,
            liquidation_cash: 0,
            pre_liq_position: 0,
            missed_ioc: 0,
            missed_ioc_buys: 0,
            missed_ioc_sells: 0,
            missed_ioc_buy_gap_sum: 0,
            missed_ioc_sell_gap_sum: 0,
            stale_fills: 0,
            stale_misses: 0,
            stale_arrival_lag_ticks_sum: 0,
            disconnected: false,
        }
    }

    fn apply_fill(&mut self, side: Side, price: u64, qty: Qty) {
        self.apply_position_delta(side, price, qty);
        match side {
            Side::Buy => {
                self.buys += 1;
                self.buy_qty += qty;
            }
            Side::Sell => {
                self.sells += 1;
                self.sell_qty += qty;
            }
        }
        self.fills += 1;
    }

    fn apply_liquidation_fill(&mut self, side: Side, price: u64, qty: Qty) {
        let old_cash = self.cash;
        self.apply_position_delta(side, price, qty);
        self.liquidation_cash += self.cash - old_cash;
        match side {
            Side::Buy => {
                self.liquidation_buys += 1;
                self.liquidation_buy_qty += qty;
                self.liquidation_buy_notional += price * qty;
            }
            Side::Sell => {
                self.liquidation_sells += 1;
                self.liquidation_sell_qty += qty;
                self.liquidation_sell_notional += price * qty;
            }
        }
        self.liquidation_fills += 1;
    }

    fn apply_position_delta(&mut self, side: Side, price: u64, qty: Qty) {
        let qty_i64 = qty as i64;
        let notional = price as i64 * qty_i64;
        match side {
            Side::Buy => {
                self.position += qty_i64;
                self.cash -= notional;
            }
            Side::Sell => {
                self.position -= qty_i64;
                self.cash += notional;
            }
        }
    }

    fn apply_stale_outcome(&mut self, outcome: StaleOutcome) {
        if outcome.filled {
            self.stale_fills += 1;
        } else {
            self.stale_misses += 1;
        }
        self.stale_arrival_lag_ticks_sum += outcome.arrival_lag_ticks;
    }
}

#[derive(Clone, Copy)]
struct PendingOrder {
    tie_breaker: u64,
    client_idx: usize,
    msg: wire::WireOrderMsg,
}

#[derive(Clone, Copy)]
enum OrderIntent {
    Normal,
    Closing,
}

impl OrderIntent {
    fn as_str(self) -> &'static str {
        match self {
            OrderIntent::Normal => "normal",
            OrderIntent::Closing => "closing",
        }
    }
}

#[derive(Clone, Copy)]
enum TradeClass {
    Close,
    Stale,
    NoStale,
}

impl TradeClass {
    fn as_str(self) -> &'static str {
        match self {
            TradeClass::Close => "close",
            TradeClass::Stale => "stale",
            TradeClass::NoStale => "no_stale",
        }
    }
}

pub fn run_server(args: &ServeArgs) {
    install_shutdown_handlers();

    let mut exchange = SimExchange::with_config(SimExchangeConfig {
        reference_event_interval: args.reference_event_interval,
        reprice_delay: args.reprice_delay,
    });
    exchange.set_debug_stale_quotes(args.debug_stale_quotes);

    let ref_udp = make_udp_sender();
    let md_udp = make_udp_sender();
    let ref_group: Ipv4Addr = args
        .ref_group
        .parse()
        .expect("invalid reference multicast group");
    let ref_addr = SocketAddr::from((ref_group, args.ref_port));
    let md_group: Ipv4Addr = args.md_group.parse().expect("invalid md multicast group");
    let md_addr = SocketAddr::from((md_group, args.md_port));

    let tcp_listener =
        TcpListener::bind(format!("0.0.0.0:{}", args.order_port)).expect("failed to bind TCP");
    tcp_listener
        .set_nonblocking(true)
        .expect("failed to set TCP non-blocking");

    let mut clients: Vec<Client> = Vec::new();
    let mut finished_clients: Vec<Client> = Vec::new();
    let mut next_client_id = 1u64;
    let mut ref_seq: u32 = 0;
    let mut md_seq: u32 = 0;
    let mut tie_rng = SmallRng::seed_from_u64(TIE_BREAKER_SEED);
    let mut pending = Vec::with_capacity(1024);
    let mut client_order = Vec::with_capacity(64);

    let tick_ns = 1_000_000_000u64 / args.tick_rate;
    let tick_interval = Duration::from_nanos(tick_ns);

    eprintln!(
        "[exchange] REF UDP {}:{}, MD UDP {}:{}, Orders TCP :{}",
        args.ref_group, args.ref_port, args.md_group, args.md_port, args.order_port
    );
    eprintln!(
        "[exchange] tick_rate={}/s  ticks={}  reference_event_interval={}  reprice_delay={}",
        args.tick_rate, args.ticks, args.reference_event_interval, args.reprice_delay
    );
    eprintln!(
        "[exchange] accepting multiple HFT clients on TCP :{}...",
        args.order_port
    );

    let t0 = Instant::now();
    let mut ticks_run = 0u64;

    while !SHUTDOWN_REQUESTED.load(Ordering::Relaxed) && (args.ticks == 0 || ticks_run < args.ticks)
    {
        let tick_start = Instant::now();

        let l1: L1 = exchange.step();

        if l1.valid() {
            let timestamp_ns = wire::now_ns();
            send_reference_snapshot(&ref_udp, ref_addr, &mut ref_seq, &l1, timestamp_ns);
            send_target_md_snapshot(&md_udp, md_addr, &mut md_seq, &l1, timestamp_ns);
        }

        accept_clients(&tcp_listener, &mut clients, &mut next_client_id);

        pending.clear();
        collect_pending_orders(&mut clients, &mut pending, &mut client_order, &mut tie_rng);
        if pending.len() > 1 {
            pending.sort_by_key(|order| order.tie_breaker);
        }

        for order in &pending {
            process_order_msg(&mut exchange, order, &mut clients);
        }

        archive_disconnected_clients(&mut exchange, &mut clients, &mut finished_clients);
        ticks_run += 1;

        let elapsed = tick_start.elapsed();
        if elapsed < tick_interval {
            std::thread::sleep(tick_interval - elapsed);
        } else if args.debug_tick_overrun {
            eprintln!(
                "[md_debug] exceeded tick interval: {:.2?} > {:.2?} (tick={})",
                elapsed, tick_interval, l1.tick
            );
        }
    }

    if SHUTDOWN_REQUESTED.load(Ordering::Relaxed) {
        eprintln!("[exchange] shutdown requested; liquidating clients and printing report...");
    }

    liquidate_clients(&mut exchange, &mut clients);
    finished_clients.append(&mut clients);

    let total = t0.elapsed();
    eprintln!(
        "[exchange] done: {} ticks in {:.2?} ({:.0} ticks/s actual)",
        ticks_run,
        total,
        ticks_run as f64 / total.as_secs_f64()
    );
    print_client_report(&finished_clients, &exchange);
}

fn make_udp_sender() -> UdpSocket {
    let udp = UdpSocket::bind("0.0.0.0:0").expect("failed to bind UDP sender");
    udp.set_multicast_loop_v4(true)
        .expect("failed to enable multicast loopback");
    udp.set_multicast_ttl_v4(1)
        .expect("failed to set multicast ttl");
    udp
}

fn send_reference_snapshot(
    udp: &UdpSocket,
    ref_addr: SocketAddr,
    seq: &mut u32,
    l1: &L1,
    timestamp_ns: u64,
) {
    if !l1.valid() {
        return;
    }

    let reference = wire::WireMdReference {
        header: wire::WireMdHeader {
            timestamp_ns,
            exchange_tick: l1.tick,
            instrument_id: wire::DEFAULT_INSTRUMENT_ID,
            sequence_num: *seq,
            msg_type: wire::MD_MSG_REFERENCE,
            _pad: [0; 7],
        },
        reference_mid: l1.reference_mid as f64,
        _pad: [0; 8],
    };
    *seq = (*seq).wrapping_add(1);
    let _ = udp.send_to(unsafe { wire::as_bytes(&reference) }, ref_addr);
}

fn send_target_md_snapshot(
    udp: &UdpSocket,
    md_addr: SocketAddr,
    seq: &mut u32,
    l1: &L1,
    timestamp_ns: u64,
) {
    if !l1.valid() {
        return;
    }

    let quote = wire::WireMdQuote {
        header: wire::WireMdHeader {
            timestamp_ns,
            exchange_tick: l1.tick,
            instrument_id: wire::DEFAULT_INSTRUMENT_ID,
            sequence_num: *seq,
            msg_type: wire::MD_MSG_QUOTE,
            _pad: [0; 7],
        },
        bid_price: l1.bid as f64,
        ask_price: l1.ask as f64,
        bid_size: l1.bid_qty as u32,
        ask_size: l1.ask_qty as u32,
    };
    *seq = (*seq).wrapping_add(1);
    let _ = udp.send_to(unsafe { wire::as_bytes(&quote) }, md_addr);
}

fn accept_clients(listener: &TcpListener, clients: &mut Vec<Client>, next_client_id: &mut u64) {
    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                stream
                    .set_nonblocking(true)
                    .expect("failed to set client non-blocking");
                stream.set_nodelay(true).ok();
                let client_id = *next_client_id;
                *next_client_id += 1;
                eprintln!("[exchange] client {} connected from {}", client_id, addr);
                clients.push(Client::new(client_id, stream));
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => {
                eprintln!("[exchange] accept error: {}", e);
                break;
            }
        }
    }
}

fn archive_disconnected_clients(
    exchange: &mut SimExchange,
    clients: &mut Vec<Client>,
    finished_clients: &mut Vec<Client>,
) {
    let mut i = 0;
    while i < clients.len() {
        if clients[i].disconnected {
            let mut client = clients.swap_remove(i);
            liquidate_client(exchange, &mut client);
            finished_clients.push(client);
        } else {
            i += 1;
        }
    }
}

fn collect_pending_orders(
    clients: &mut [Client],
    pending: &mut Vec<PendingOrder>,
    client_order: &mut Vec<usize>,
    tie_rng: &mut SmallRng,
) {
    client_order.clear();
    client_order.extend(0..clients.len());
    if client_order.len() > 1 {
        client_order.shuffle(tie_rng);
    }

    let mut tmp = [0u8; 4096];
    for &client_idx in client_order.iter() {
        let client = &mut clients[client_idx];
        loop {
            match client.stream.read(&mut tmp) {
                Ok(0) => {
                    eprintln!("[exchange] client {} disconnected", client.id);
                    client.disconnected = true;
                    break;
                }
                Ok(n) => client.read_buf.extend_from_slice(&tmp[..n]),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    eprintln!("[exchange] client {} read error: {}", client.id, e);
                    client.disconnected = true;
                    break;
                }
            }
        }

        let msg_size = size_of::<wire::WireOrderMsg>();
        let parsed_len = client.read_buf.len() / msg_size * msg_size;
        let mut offset = 0;
        while offset < parsed_len {
            let msg = unsafe { wire::from_bytes::<wire::WireOrderMsg>(&client.read_buf[offset..]) };
            pending.push(PendingOrder {
                tie_breaker: tie_rng.gen(),
                client_idx,
                msg,
            });
            offset += msg_size;
        }
        if parsed_len > 0 {
            client.read_buf.drain(..parsed_len);
        }
    }
}

fn process_order_msg(exchange: &mut SimExchange, order: &PendingOrder, clients: &mut [Client]) {
    if order.client_idx >= clients.len() || clients[order.client_idx].disconnected {
        return;
    }

    if order.msg.msg_type == wire::ORDER_MSG_NEW {
        process_new_order(exchange, order, clients);
    } else {
        eprintln!(
            "[exchange] client {} unknown order msg_type: {}",
            clients[order.client_idx].id, order.msg.msg_type
        );
    }
}

fn process_new_order(exchange: &mut SimExchange, order: &PendingOrder, clients: &mut [Client]) {
    let msg = order.msg;
    let side = wire::wire_to_side(msg.side);
    let price = msg.price.round() as u64;
    let qty = msg.qty as Qty;
    let pre_order_position = clients[order.client_idx].position;
    let order_intent = classify_order_intent(pre_order_position, side, qty);

    clients[order.client_idx].orders_total += 1;

    let (fills, stale_outcome) = match msg.order_type {
        wire::ORDER_TYPE_IOC_LIMIT => {
            let result = exchange.submit_ioc_limit_at(side, price, qty);
            (result.fills, result.stale_outcome)
        }
        wire::ORDER_TYPE_MARKET => (exchange.submit_market(side, qty), None),
        _ => {
            let client = &mut clients[order.client_idx];
            client.orders_rejected += 1;
            send_exec_report(
                client,
                wire::WireExecReport {
                    exec_type: wire::EXEC_REJECT,
                    side: msg.side,
                    _pad1: [0; 2],
                    fill_qty: 0,
                    order_id: msg.client_order_id,
                    fill_price: 0.0,
                    leaves_qty: qty as u32,
                    _pad2: 0,
                    timestamp_ns: wire::now_ns(),
                },
            );
            return;
        }
    };

    if exchange.debug_stale_quotes_enabled() && !fills.is_empty() {
        log_exchange_fills(
            exchange,
            clients[order.client_idx].id,
            &msg,
            side,
            price,
            qty,
            &fills,
            stale_outcome,
            pre_order_position,
            order_intent,
        );
    }

    let client = &mut clients[order.client_idx];
    client.orders_accepted += 1;
    if let Some(outcome) = stale_outcome {
        client.apply_stale_outcome(outcome);
    }
    if fills.is_empty() && msg.order_type == wire::ORDER_TYPE_IOC_LIMIT {
        let l1 = exchange.debug_l1();
        let gap = match side {
            Side::Buy => l1.ask.saturating_sub(price),
            Side::Sell => price.saturating_sub(l1.bid),
        };
        match side {
            Side::Buy => {
                client.missed_ioc_buys += 1;
                client.missed_ioc_buy_gap_sum += gap;
            }
            Side::Sell => {
                client.missed_ioc_sells += 1;
                client.missed_ioc_sell_gap_sum += gap;
            }
        }
        client.missed_ioc += 1;
    } else if !fills.is_empty() {
        client.orders_filled += 1;
    }

    send_exec_report(
        client,
        wire::WireExecReport {
            exec_type: wire::EXEC_NEW_ACK,
            side: msg.side,
            _pad1: [0; 2],
            fill_qty: 0,
            order_id: msg.client_order_id,
            fill_price: 0.0,
            leaves_qty: qty as u32,
            _pad2: 0,
            timestamp_ns: wire::now_ns(),
        },
    );

    let mut filled_so_far: Qty = 0;
    for fill in &fills {
        filled_so_far += fill.qty;
        client.apply_fill(side, fill.price, fill.qty);
        let leaves = qty.saturating_sub(filled_so_far);
        let exec_type = if leaves == 0 {
            wire::EXEC_FILL
        } else {
            wire::EXEC_PARTIAL_FILL
        };
        send_exec_report(
            client,
            wire::WireExecReport {
                exec_type,
                side: msg.side,
                _pad1: [0; 2],
                fill_qty: fill.qty as u32,
                order_id: msg.client_order_id,
                fill_price: fill.price as f64,
                leaves_qty: leaves as u32,
                _pad2: 0,
                timestamp_ns: wire::now_ns(),
            },
        );
    }

    if msg.order_type == wire::ORDER_TYPE_IOC_LIMIT {
        let leaves = qty.saturating_sub(filled_so_far);
        if leaves > 0 {
            send_exec_report(
                client,
                wire::WireExecReport {
                    exec_type: wire::EXEC_CANCEL_ACK,
                    side: msg.side,
                    _pad1: [0; 2],
                    fill_qty: 0,
                    order_id: msg.client_order_id,
                    fill_price: 0.0,
                    leaves_qty: leaves as u32,
                    _pad2: 0,
                    timestamp_ns: wire::now_ns(),
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn log_exchange_fills(
    exchange: &SimExchange,
    client_id: u64,
    msg: &wire::WireOrderMsg,
    side: Side,
    limit_price: u64,
    order_qty: Qty,
    fills: &[Fill],
    stale_outcome: Option<StaleOutcome>,
    pre_order_position: i64,
    order_intent: OrderIntent,
) {
    let l1 = exchange.debug_l1();
    let kind = fill_log_kind(stale_outcome, order_intent);
    let class = trade_class(stale_outcome, order_intent);
    let stale_event = stale_outcome.map(|outcome| outcome.event_id).unwrap_or(0);

    for fill in fills {
        eprintln!(
            "------[trade]------ class={} kind={} intent={} tick={} client={} order={} pre_pos={} side={:?} limit={} order_qty={} fill_price={} fill_qty={} stale_event={} ref={} bid={} ask={}",
            class.as_str(),
            kind,
            order_intent.as_str(),
            exchange.tick(),
            client_id,
            msg.client_order_id,
            pre_order_position,
            side,
            limit_price,
            order_qty,
            fill.price,
            fill.qty,
            stale_event,
            l1.reference_mid,
            l1.bid,
            l1.ask
        );
    }
}

fn trade_class(stale_outcome: Option<StaleOutcome>, order_intent: OrderIntent) -> TradeClass {
    match (stale_outcome, order_intent) {
        (_, OrderIntent::Closing) => TradeClass::Close,
        (Some(_), OrderIntent::Normal) => TradeClass::Stale,
        (None, OrderIntent::Normal) => TradeClass::NoStale,
    }
}

fn classify_order_intent(position: i64, side: Side, qty: Qty) -> OrderIntent {
    let qty = qty as i64;
    let closes_existing_position = match side {
        Side::Buy => position < 0 && qty <= -position,
        Side::Sell => position > 0 && qty <= position,
    };
    if closes_existing_position {
        OrderIntent::Closing
    } else {
        OrderIntent::Normal
    }
}

fn fill_log_kind(stale_outcome: Option<StaleOutcome>, order_intent: OrderIntent) -> &'static str {
    match (stale_outcome, order_intent) {
        (Some(outcome), OrderIntent::Closing) if outcome.filled => "stale_closing_fill",
        (Some(_), OrderIntent::Closing) => "stale_closing_attempt_other_fill",
        (None, OrderIntent::Closing) => "closing_fill",
        (Some(outcome), OrderIntent::Normal) if outcome.filled => "stale_fill",
        (Some(_), OrderIntent::Normal) => "stale_attempt_other_fill",
        (None, OrderIntent::Normal) => "non_stale_fill",
    }
}

fn send_exec_report(client: &mut Client, report: wire::WireExecReport) {
    if client
        .stream
        .write_all(unsafe { wire::as_bytes(&report) })
        .is_err()
    {
        client.disconnected = true;
    }
}

fn liquidate_clients(exchange: &mut SimExchange, clients: &mut [Client]) {
    for client in clients {
        liquidate_client(exchange, client);
    }
}

fn liquidate_client(exchange: &mut SimExchange, client: &mut Client) {
    client.pre_liq_position = client.position;
    let position = client.position;
    let l1 = exchange.debug_l1();
    let mark_price = if l1.valid() {
        (l1.bid + l1.ask) as f64 * 0.5
    } else {
        0.0
    };
    let pre_liq_mark_pnl = position as f64 * mark_price;
    let old_liquidation_cash = client.liquidation_cash;

    if position > 0 {
        let fills = exchange.submit_market(Side::Sell, position as Qty);
        for fill in fills {
            client.apply_liquidation_fill(Side::Sell, fill.price, fill.qty);
        }
    } else if position < 0 {
        let fills = exchange.submit_market(Side::Buy, (-position) as Qty);
        for fill in fills {
            client.apply_liquidation_fill(Side::Buy, fill.price, fill.qty);
        }
    }

    let liquidation_cash_delta = client.liquidation_cash - old_liquidation_cash;
    client.liquidation_pnl += liquidation_cash_delta as f64 - pre_liq_mark_pnl;
}

fn print_client_report(clients: &[Client], exchange: &SimExchange) {
    eprintln!("\n=== Exchange Client Report ===");
    let stale_fills: u64 = clients.iter().map(|client| client.stale_fills).sum();
    let stale_misses: u64 = clients.iter().map(|client| client.stale_misses).sum();
    let stale_attempts = stale_fills + stale_misses;
    let stale_capture_rate = if exchange.stale_events() > 0 {
        100.0 * stale_fills as f64 / exchange.stale_events() as f64
    } else {
        0.0
    };
    let stale_lag_sum: u64 = clients
        .iter()
        .map(|client| client.stale_arrival_lag_ticks_sum)
        .sum();
    let avg_arrival_lag_ticks = if stale_attempts > 0 {
        stale_lag_sum as f64 / stale_attempts as f64
    } else {
        0.0
    };
    eprintln!(
        "Stale: events {}, fills {}, misses {}, expired {}, capture {:.1}%, avg arrival lag {:.2} ticks",
        exchange.stale_events(),
        stale_fills,
        stale_misses,
        exchange.expired_stale_quotes(),
        stale_capture_rate,
        avg_arrival_lag_ticks
    );
    for client in clients {
        let hit_rate = if client.orders_accepted > 0 {
            100.0 * client.orders_filled as f64 / client.orders_accepted as f64
        } else {
            0.0
        };
        let total_pnl = client.cash as f64;
        let realized_pnl = total_pnl - client.liquidation_pnl;
        let liq_buy_avg = if client.liquidation_buy_qty > 0 {
            client.liquidation_buy_notional as f64 / client.liquidation_buy_qty as f64
        } else {
            0.0
        };
        let liq_sell_avg = if client.liquidation_sell_qty > 0 {
            client.liquidation_sell_notional as f64 / client.liquidation_sell_qty as f64
        } else {
            0.0
        };
        let missed_buy_avg_gap = if client.missed_ioc_buys > 0 {
            client.missed_ioc_buy_gap_sum as f64 / client.missed_ioc_buys as f64
        } else {
            0.0
        };
        let missed_sell_avg_gap = if client.missed_ioc_sells > 0 {
            client.missed_ioc_sell_gap_sum as f64 / client.missed_ioc_sells as f64
        } else {
            0.0
        };
        let stale_attempts = client.stale_fills + client.stale_misses;
        let stale_capture_rate = if exchange.stale_events() > 0 {
            100.0 * client.stale_fills as f64 / exchange.stale_events() as f64
        } else {
            0.0
        };
        let avg_arrival_lag_ticks = if stale_attempts > 0 {
            client.stale_arrival_lag_ticks_sum as f64 / stale_attempts as f64
        } else {
            0.0
        };
        eprintln!("--- Client {} ---", client.id);
        eprintln!(
            "  Orders:      {} total, {} accepted, {} filled, {} missed IOC, {} rejected ({:.1}% hit)",
            client.orders_total,
            client.orders_accepted,
            client.orders_filled,
            client.missed_ioc,
            client.orders_rejected,
            hit_rate
        );
        eprintln!(
            "  Miss Debug:  {} buy misses (avg gap {:.2}), {} sell misses (avg gap {:.2})",
            client.missed_ioc_buys,
            missed_buy_avg_gap,
            client.missed_ioc_sells,
            missed_sell_avg_gap
        );
        eprintln!(
            "  Stale:       {} fills, {} misses, capture {:.1}%, avg arrival lag {:.2} ticks",
            client.stale_fills, client.stale_misses, stale_capture_rate, avg_arrival_lag_ticks
        );
        eprintln!(
            "  Fills:       {} events, {} qty ({} buys / {} buy qty, {} sells / {} sell qty)",
            client.fills,
            client.buy_qty + client.sell_qty,
            client.buys,
            client.buy_qty,
            client.sells,
            client.sell_qty
        );
        eprintln!(
            "  Liquidation: pos {:+} -> {:+}, {} fills, {} qty",
            client.pre_liq_position,
            client.position,
            client.liquidation_fills,
            client.liquidation_buy_qty + client.liquidation_sell_qty
        );
        eprintln!(
            "    Buy:       {} fills, {} qty @ {:.2}",
            client.liquidation_buys, client.liquidation_buy_qty, liq_buy_avg
        );
        eprintln!(
            "    Sell:      {} fills, {} qty @ {:.2}",
            client.liquidation_sells, client.liquidation_sell_qty, liq_sell_avg
        );
        eprintln!(
            "  PnL:         realized {:+.2}, liquidation {:+.2}, total {:+.2}",
            realized_pnl, client.liquidation_pnl, total_pnl
        );
    }
}
