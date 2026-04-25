use std::io::{self, Read, Write};
use std::mem::size_of;
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use matching_engine::{OrderType, Qty, Side};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use super::wire;
use crate::arg::ServeArgs;
use crate::sim::exchange::SimExchange;

struct Client {
    id: u64,
    stream: TcpStream,
    read_buf: Vec<u8>,
    position: i64,
    cash: i64,
    fills: u64,
    buys: u64,
    sells: u64,
    missed_ioc: u64,
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
            fills: 0,
            buys: 0,
            sells: 0,
            missed_ioc: 0,
            disconnected: false,
        }
    }

    fn apply_fill(&mut self, side: Side, price: u64, qty: Qty) {
        let qty_i64 = qty as i64;
        let notional = price as i64 * qty_i64;
        match side {
            Side::Buy => {
                self.position += qty_i64;
                self.cash -= notional;
                self.buys += 1;
            }
            Side::Sell => {
                self.position -= qty_i64;
                self.cash += notional;
                self.sells += 1;
            }
        }
        self.fills += 1;
    }
}

#[derive(Clone, Copy)]
struct PendingOrder {
    recv_ns: u64,
    tie_breaker: u64,
    client_idx: usize,
    msg: wire::WireOrderMsg,
}

pub fn run_server(args: &ServeArgs) {
    let mut exchange = SimExchange::new(args.seed);

    let udp = UdpSocket::bind("0.0.0.0:0").expect("failed to bind UDP sender");
    let md_addr: SocketAddr = format!("127.0.0.1:{}", args.md_port)
        .parse()
        .expect("invalid md address");

    let tcp_listener =
        TcpListener::bind(format!("0.0.0.0:{}", args.order_port)).expect("failed to bind TCP");
    tcp_listener
        .set_nonblocking(true)
        .expect("failed to set TCP non-blocking");

    let mut clients: Vec<Client> = Vec::new();
    let mut next_client_id = 1u64;
    let mut seq: u32 = 0;
    let mut tie_rng = SmallRng::seed_from_u64(args.seed ^ 0x9e37_79b9_7f4a_7c15);

    let tick_ns = 1_000_000_000u64 / args.tick_rate;
    let tick_interval = Duration::from_nanos(tick_ns);

    eprintln!(
        "[exchange] MD UDP :{}, Orders TCP :{}",
        args.md_port, args.order_port
    );
    eprintln!(
        "[exchange] tick_rate={}/s  ticks={}  seed={}",
        args.tick_rate, args.ticks, args.seed
    );
    eprintln!(
        "[exchange] accepting multiple HFT clients on TCP :{}...",
        args.order_port
    );

    let t0 = Instant::now();

    for _tick in 0..args.ticks {
        let tick_start = Instant::now();

        let l1 = exchange.step();

        if l1.valid() {
            let now_ns = wire::now_ns();
            let reference = wire::WireMdReference {
                header: wire::WireMdHeader {
                    timestamp_ns: now_ns,
                    instrument_id: wire::DEFAULT_INSTRUMENT_ID,
                    sequence_num: seq,
                    msg_type: wire::MD_MSG_REFERENCE,
                    _pad: [0; 7],
                },
                reference_mid: l1.reference_mid as f64,
                _pad: [0; 8],
            };
            seq = seq.wrapping_add(1);
            let _ = udp.send_to(unsafe { wire::as_bytes(&reference) }, md_addr);

            let quote = wire::WireMdQuote {
                header: wire::WireMdHeader {
                    timestamp_ns: wire::now_ns(),
                    instrument_id: wire::DEFAULT_INSTRUMENT_ID,
                    sequence_num: seq,
                    msg_type: wire::MD_MSG_QUOTE,
                    _pad: [0; 7],
                },
                bid_price: l1.bid as f64,
                ask_price: l1.ask as f64,
                bid_size: l1.bid_qty as u32,
                ask_size: l1.ask_qty as u32,
            };
            seq = seq.wrapping_add(1);
            let _ = udp.send_to(unsafe { wire::as_bytes(&quote) }, md_addr);
        }

        accept_clients(&tcp_listener, &mut clients, &mut next_client_id);

        let mut pending = Vec::new();
        collect_pending_orders(&mut clients, &mut pending, &mut tie_rng);
        pending.sort_by_key(|order| (order.recv_ns, order.tie_breaker));

        for order in pending {
            process_order_msg(&mut exchange, &order, &mut clients);
        }

        clients.retain(|client| !client.disconnected);

        let elapsed = tick_start.elapsed();
        if elapsed < tick_interval {
            std::thread::sleep(tick_interval - elapsed);
        }
    }

    liquidate_clients(&mut exchange, &mut clients);

    let total = t0.elapsed();
    eprintln!(
        "[exchange] done: {} ticks in {:.2?} ({:.0} ticks/s actual)",
        args.ticks,
        total,
        args.ticks as f64 / total.as_secs_f64()
    );
    print_client_report(&clients);
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

fn collect_pending_orders(
    clients: &mut [Client],
    pending: &mut Vec<PendingOrder>,
    tie_rng: &mut SmallRng,
) {
    for (client_idx, client) in clients.iter_mut().enumerate() {
        let mut tmp = [0u8; 4096];
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

        while client.read_buf.len() >= size_of::<wire::WireOrderMsg>() {
            let recv_ns = wire::now_ns();
            let msg = unsafe { wire::from_bytes::<wire::WireOrderMsg>(&client.read_buf) };
            client.read_buf.drain(..size_of::<wire::WireOrderMsg>());
            pending.push(PendingOrder {
                recv_ns,
                tie_breaker: tie_rng.gen(),
                client_idx,
                msg,
            });
        }
    }
}

fn process_order_msg(exchange: &mut SimExchange, order: &PendingOrder, clients: &mut [Client]) {
    if order.client_idx >= clients.len() || clients[order.client_idx].disconnected {
        return;
    }

    match order.msg.msg_type {
        wire::ORDER_MSG_NEW => process_new_order(exchange, order, clients),
        wire::ORDER_MSG_CANCEL => process_cancel_order(exchange, order, clients),
        _ => eprintln!(
            "[exchange] client {} unknown order msg_type: {}",
            clients[order.client_idx].id, order.msg.msg_type
        ),
    }
}

fn process_new_order(exchange: &mut SimExchange, order: &PendingOrder, clients: &mut [Client]) {
    let msg = order.msg;
    let side = wire::wire_to_side(msg.side);
    let price = msg.price.round() as u64;
    let qty = msg.qty as Qty;

    let fills = match msg.order_type {
        wire::ORDER_TYPE_IOC_LIMIT => exchange.submit_ioc_limit(side, price, qty),
        wire::ORDER_TYPE_LIMIT => {
            exchange.submit_hft_order(msg.client_order_id, side, price, qty, OrderType::Limit)
        }
        wire::ORDER_TYPE_MARKET => {
            exchange.submit_hft_order(msg.client_order_id, side, price, qty, OrderType::Market)
        }
        _ => {
            send_exec_report(
                &mut clients[order.client_idx],
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

    let client = &mut clients[order.client_idx];
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

    if fills.is_empty() && msg.order_type == wire::ORDER_TYPE_IOC_LIMIT {
        client.missed_ioc += 1;
    }

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
}

fn process_cancel_order(exchange: &mut SimExchange, order: &PendingOrder, clients: &mut [Client]) {
    let msg = order.msg;
    let success = exchange.cancel_hft_order(msg.cancel_order_id);
    let exec_type = if success {
        wire::EXEC_CANCEL_ACK
    } else {
        wire::EXEC_CANCEL_REJECT
    };
    send_exec_report(
        &mut clients[order.client_idx],
        wire::WireExecReport {
            exec_type,
            side: 0,
            _pad1: [0; 2],
            fill_qty: 0,
            order_id: msg.cancel_order_id,
            fill_price: 0.0,
            leaves_qty: 0,
            _pad2: 0,
            timestamp_ns: wire::now_ns(),
        },
    );
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
        let position = client.position;
        if position > 0 {
            let fills = exchange.submit_market(Side::Sell, position as Qty);
            for fill in fills {
                client.apply_fill(Side::Sell, fill.price, fill.qty);
            }
        } else if position < 0 {
            let fills = exchange.submit_market(Side::Buy, (-position) as Qty);
            for fill in fills {
                client.apply_fill(Side::Buy, fill.price, fill.qty);
            }
        }
    }
}

fn print_client_report(clients: &[Client]) {
    eprintln!("\n=== Exchange Client PnL Report ===");
    for client in clients {
        eprintln!("--- Client {} ---", client.id);
        eprintln!(
            "  Fills:       {} ({} buys, {} sells)",
            client.fills, client.buys, client.sells
        );
        eprintln!("  Missed IOC:  {}", client.missed_ioc);
        eprintln!("  Position:    {:+}", client.position);
        eprintln!("  Cash/PnL:    {:+}", client.cash);
    }
}
