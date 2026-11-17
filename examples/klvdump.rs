//! Prints every ST 0601 packet in a KLV data stream, one line per item.
//!
//! Get the KLV stream out of a STANAG 4609 video first:
//!
//! ```text
//! ffmpeg -i "Day Flight.mpg" -map 0:d -c copy -f data day_flight.klv
//! cargo run --example klvdump -- day_flight.klv
//! ```
use misb::st0601::{Packet, Value};
use misb::{klv, st0601};

fn main() {
    let path = std::env::args().nth(1).expect("usage: klvdump <file.klv>");
    let bytes = std::fs::read(&path).expect("read input");
    for (n, unit) in klv::split(&bytes).enumerate() {
        if unit.key != &st0601::KEY {
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
    }
}
