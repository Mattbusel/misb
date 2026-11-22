//! Prints every ST 0601 packet in a KLV data stream or a transport
//! stream, one line per item.
//!
//! ```text
//! cargo run --example klvdump -- "Day Flight.mpg"
//! cargo run --example klvdump -- day_flight.klv
//! ```
//!
//! For a transport stream each packet is shown with its PID and PTS, so
//! you can line it up with the video frame it describes.
use misb::st0601::{Packet, Value};
use misb::{klv, st0601, ts};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: klvdump <file.ts|file.klv>");
    let bytes = std::fs::read(&path).expect("read input");
    if !bytes.starts_with(&klv::UL_PREFIX) && ts::is_transport_stream(&bytes) {
        for unit in ts::klv_units(&bytes) {
            let pts = unit
                .pts_seconds()
                .map_or("no PTS".to_string(), |s| format!("PTS {s:.3} s"));
            println!("PID {:#06x}, {pts}", unit.pid);
            dump(&unit.data);
        }
    } else {
        dump(&bytes);
    }
}

fn dump(bytes: &[u8]) {
    for (n, unit) in klv::split(bytes).enumerate() {
        if !klv::same_key(unit.key, &st0601::KEY) {
            println!("#{n}: other KLV key {:02x?}", unit.key);
            continue;
        }
        let packet = match Packet::from_unit(&unit) {
            Ok(p) => p,
            Err(e) => {
                println!("#{n}: {e}");
                continue;
            }
        };
        println!("#{n}: {} bytes", unit.raw.len());
        for item in packet.items() {
            let item = match item {
                Ok(i) => i,
                Err(e) => {
                    println!("    malformed item: {e}");
                    break;
                }
            };
            let units = st0601::tag_info(item.tag).map_or("", |t| t.units);
            match item.value() {
                Value::Float(v) => println!("  {:>3} {:<42} {v:.7} {units}", item.tag, item.name()),
                Value::Security(s) => {
                    println!(
                        "  {:>3} {:<42} {:?}",
                        item.tag,
                        item.name(),
                        s.classification()
                    );
                }
                Value::Vmti(v) => {
                    println!(
                        "  {:>3} {:<42} {} targets",
                        item.tag,
                        item.name(),
                        v.targets().count()
                    );
                }
                v => println!("  {:>3} {:<42} {v:?} {units}", item.tag, item.name()),
            }
        }
        if let Some(c) = packet.corners() {
            println!("  footprint: {:?}", c.map(|p| (p.lat, p.lon)));
        }
        if let Some(f) = packet.frame_rate().and_then(|f| f.fps()) {
            println!("  frame rate: {f:.3} fps");
        }
        if let Some(a) = packet.airbase_locations() {
            println!("  airbase: {a:?}");
        }
        if let Some(p) = packet.payloads() {
            println!("  payloads: {p:?}");
        }
        if let Some(w) = packet.waypoints() {
            println!("  waypoints: {w:?}");
        }
        for c in packet.control_commands() {
            println!("  command: {c:?}");
        }
    }
}
