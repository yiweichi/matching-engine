use std::io::{self, Read, Write};
use std::mem::size_of;
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use matching_engine::{OrderType, Qty};

use super::wire;
use crate::arg::ServeArgs;
use crate::sim::exchange::SimExchange;

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

    let mut client: Option<TcpStream> = None;
    let mut read_buf: Vec<u8> = Vec::with_capacity(4096);
    let mut seq: u32 = 0;

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
    eprintln!("[exchange] waiting for HFT client on TCP :{}...", args.order_port);

    let t0 = Instant::now();

    for _tick in 0..args.ticks {
        let tick_start = Instant::now();

        // 1. Step exchange (noise + book update)
        let l1 = exchange.step();

        // 2. Broadcast L1 market data via UDP
        if l1.valid() {
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
            seq += 1;
            let bytes = unsafe { wire::as_bytes(&quote) };
            let _ = udp.send_to(bytes, md_addr);
        }

        // 3. Accept new TCP client
        if client.is_none() {
            match tcp_listener.accept() {
                Ok((stream, addr)) => {
                    stream
                        .set_nonblocking(true)
                        .expect("failed to set client non-blocking");
                    stream.set_nodelay(true).ok();
                    eprintln!("[exchange] HFT client connected from {}", addr);
                    client = Some(stream);
                    read_buf.clear();
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => eprintln!("[exchange] accept error: {}", e),
            }
        }

        // 4. Read incoming bytes from TCP client
        let mut client_lost = false;
        if let Some(ref mut stream) = client {
            let mut tmp = [0u8; 4096];
            match stream.read(&mut tmp) {
                Ok(0) => {
                    eprintln!("[exchange] HFT client disconnected");
                    client_lost = true;
                }
                Ok(n) => read_buf.extend_from_slice(&tmp[..n]),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("[exchange] read error: {}", e);
                    client_lost = true;
                }
            }
        }
        if client_lost {
            client = None;
            read_buf.clear();
        }

        // 5. Process complete order messages
        if let Some(ref mut stream) = client {
            while read_buf.len() >= size_of::<wire::WireOrderMsg>() {
                let msg = unsafe { wire::from_bytes::<wire::WireOrderMsg>(&read_buf) };
                read_buf.drain(..size_of::<wire::WireOrderMsg>());
                process_order_msg(&mut exchange, &msg, stream);
            }
        }

        // 6. Send passive fill reports (from noise filling HFT resting orders)
        let reports = exchange.drain_hft_reports();
        if !reports.is_empty() {
            if let Some(ref mut stream) = client {
                for report in &reports {
                    let is_full = report.leaves_qty == 0;
                    let wire_report = wire::WireExecReport {
                        exec_type: if is_full {
                            wire::EXEC_FILL
                        } else {
                            wire::EXEC_PARTIAL_FILL
                        },
                        side: wire::side_to_wire(report.side),
                        _pad1: [0; 2],
                        fill_qty: report.qty as u32,
                        order_id: report.order_id,
                        fill_price: report.price as f64,
                        leaves_qty: report.leaves_qty as u32,
                        _pad2: 0,
                        timestamp_ns: wire::now_ns(),
                    };
                    let bytes = unsafe { wire::as_bytes(&wire_report) };
                    let _ = stream.write_all(bytes);
                }
            }
        }

        // 7. Sleep for remaining tick duration
        let elapsed = tick_start.elapsed();
        if elapsed < tick_interval {
            std::thread::sleep(tick_interval - elapsed);
        }
    }

    let total = t0.elapsed();
    eprintln!(
        "[exchange] done: {} ticks in {:.2?} ({:.0} ticks/s actual)",
        args.ticks,
        total,
        args.ticks as f64 / total.as_secs_f64()
    );
}

fn process_order_msg(exchange: &mut SimExchange, msg: &wire::WireOrderMsg, stream: &mut TcpStream) {
    match msg.msg_type {
        wire::ORDER_MSG_NEW => {
            let side = wire::wire_to_side(msg.side);
            let order_type = if msg.order_type == wire::ORDER_TYPE_LIMIT {
                OrderType::Limit
            } else {
                OrderType::Market
            };
            let price = msg.price.round() as u64;
            let qty = msg.qty as Qty;

            let fills = exchange.submit_hft_order(msg.client_order_id, side, price, qty, order_type);

            // NewAck
            let ack = wire::WireExecReport {
                exec_type: wire::EXEC_NEW_ACK,
                side: msg.side,
                _pad1: [0; 2],
                fill_qty: 0,
                order_id: msg.client_order_id,
                fill_price: 0.0,
                leaves_qty: qty as u32,
                _pad2: 0,
                timestamp_ns: wire::now_ns(),
            };
            let _ = stream.write_all(unsafe { wire::as_bytes(&ack) });

            // Immediate fills
            let mut filled_so_far: Qty = 0;
            for fill in &fills {
                filled_so_far += fill.qty;
                let leaves = qty.saturating_sub(filled_so_far);
                let exec_type = if leaves == 0 {
                    wire::EXEC_FILL
                } else {
                    wire::EXEC_PARTIAL_FILL
                };
                let report = wire::WireExecReport {
                    exec_type,
                    side: msg.side,
                    _pad1: [0; 2],
                    fill_qty: fill.qty as u32,
                    order_id: msg.client_order_id,
                    fill_price: fill.price as f64,
                    leaves_qty: leaves as u32,
                    _pad2: 0,
                    timestamp_ns: wire::now_ns(),
                };
                let _ = stream.write_all(unsafe { wire::as_bytes(&report) });
            }
        }
        wire::ORDER_MSG_CANCEL => {
            let success = exchange.cancel_hft_order(msg.cancel_order_id);
            let exec_type = if success {
                wire::EXEC_CANCEL_ACK
            } else {
                wire::EXEC_CANCEL_REJECT
            };
            let report = wire::WireExecReport {
                exec_type,
                side: 0,
                _pad1: [0; 2],
                fill_qty: 0,
                order_id: msg.cancel_order_id,
                fill_price: 0.0,
                leaves_qty: 0,
                _pad2: 0,
                timestamp_ns: wire::now_ns(),
            };
            let _ = stream.write_all(unsafe { wire::as_bytes(&report) });
        }
        _ => {
            eprintln!("[exchange] unknown order msg_type: {}", msg.msg_type);
        }
    }
}
