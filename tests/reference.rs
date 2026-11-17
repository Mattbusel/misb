//! Checks the decoder against the reference implementation and the MISB
//! sample values.
//!
//! * `tests/data/day_flight.klv` and `night_flight_ir.klv` are the KLV data
//!   streams of the public "Day Flight.mpg" and "Night Flight IR.mpg"
//!   samples (samples.ffmpeg.org/MPEG2/mpegts-klv), extracted with
//!   `ffmpeg -i <file> -map 0:d -c copy -f data <out>`.
//! * `Dynamic*MISMMSPacketData.bin` are the MISB sample packets shipped with
//!   paretech/klvdata (MIT).
//! * `klvdata_golden.jsonl` is what paretech/klvdata decodes from those files
//!   (`tools/golden.py`): every tag value of every packet.

use misb::st0601::{Packet, Tag, Value};
use misb::{klv, st0102};
use serde_json::Value as Json;

const FILES: &[&str] = &[
    "day_flight.klv",
    "night_flight_ir.klv",
    "DynamicConstantMISMMSPacketData.bin",
    "DynamicOnlyMISMMSPacketData.bin",
];

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The MISB "Dynamic and Constant" sample was edited after its checksum was
/// computed (see `sample_with_stale_checksum_is_flagged`); read it unverified.
fn parse<'a>(file: &str, unit: &klv::Unit<'a>) -> Packet<'a> {
    if file == "DynamicConstantMISMMSPacketData.bin" {
        Packet::parse_unverified(unit.raw).unwrap().0
    } else {
        Packet::from_unit(unit).unwrap_or_else(|e| panic!("{file}: {e}"))
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn golden() -> Vec<(String, serde_json::Map<String, Json>)> {
    let text = std::fs::read_to_string(format!(
        "{}/tests/data/klvdata_golden.jsonl",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    text.lines()
        .map(|l| {
            let v: Json = serde_json::from_str(l).unwrap();
            (
                v["file"].as_str().unwrap().to_string(),
                v["tags"].as_object().unwrap().clone(),
            )
        })
        .collect()
}

/// Every packet in the samples has a valid checksum and decodes to the same
/// value as klvdata for every tag klvdata knows.
#[test]
fn matches_klvdata_on_every_tag() {
    let golden = golden();
    let mut g = golden.iter();
    let mut packets = 0;
    let mut compared = 0;
    for file in FILES {
        let bytes = data(file);
        for unit in klv::split(&bytes) {
            let packet = parse(file, &unit);
            let (gfile, tags) = g.next().expect("klvdata decoded fewer packets");
            assert_eq!(gfile, file);
            packets += 1;
            for (tag, want) in tags {
                let tag: u32 = tag.parse().unwrap();
                let item = packet.items().flatten().find(|i| i.tag == tag).unwrap();
                match (item.value(), want) {
                    (Value::Float(v), Json::Number(n)) => {
                        let n = n.as_f64().unwrap();
                        assert!(
                            (v - n).abs() <= 1e-9 * n.abs().max(1.0),
                            "{file} tag {tag}: {v} vs klvdata {n}"
                        );
                    }
                    (Value::Uint(v), Json::Number(n)) => {
                        assert_eq!(v as f64, n.as_f64().unwrap(), "{file} tag {tag}")
                    }
                    (Value::Time(v), Json::Number(n)) => {
                        assert_eq!(v, n.as_u64().unwrap(), "{file} tag {tag}")
                    }
                    (Value::Text(s), Json::String(w)) => assert_eq!(s, w, "{file} tag {tag}"),
                    (Value::Bytes(b), Json::String(w)) => {
                        assert_eq!(&hex(b), w, "{file} tag {tag}")
                    }
                    (Value::Uint(v), Json::String(w)) if tag == 1 => {
                        assert_eq!(format!("{v:04x}"), *w)
                    }
                    (Value::Security(s), Json::Object(m)) => {
                        for (st, sv) in m {
                            let st: u32 = st.parse().unwrap();
                            let raw = s.items().flatten().find(|i| i.tag == st).unwrap().value;
                            assert_eq!(&hex(raw), sv, "{file} 0102 tag {st}");
                        }
                    }
                    (got, want) => panic!("{file} tag {tag}: {got:?} vs klvdata {want}"),
                }
                compared += 1;
            }
        }
    }
    assert!(g.next().is_none(), "klvdata decoded more packets");
    assert_eq!(packets, 26);
    assert!(compared > 500, "{compared}");
}

/// The MISB sample packet carries the values of the ST 0601 example
/// ("Mission 12", "Predator"); each decodes to within one quantisation step.
#[test]
fn st0601_sample_values() {
    let bytes = data("DynamicConstantMISMMSPacketData.bin");
    let (p, used) = Packet::parse_unverified(&bytes).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(p.timestamp(), Some(1_231_798_102_000_000)); // 2009-01-12 22:08:22 UTC
    assert_eq!(p.text(Tag::MissionId), Some("Mission 12"));
    assert_eq!(p.text(Tag::PlatformDesignation), Some("Predator"));
    let close = |t: Tag, want: f64, step: f64| {
        let v = p.float(t).unwrap();
        assert!((v - want).abs() <= step, "{t:?}: {v} vs {want}");
    };
    close(Tag::PlatformHeadingAngle, 159.9744, 360.0 / 65535.0);
    close(Tag::PlatformPitchAngle, -0.4315251, 40.0 / 65534.0);
    close(Tag::PlatformRollAngle, 3.405814, 100.0 / 65534.0);
    close(Tag::SensorLatitude, 60.1768229669783, 180.0 / 4294967294.0);
    close(Tag::SensorLongitude, 128.426759042045, 360.0 / 4294967294.0);
    close(Tag::SensorTrueAltitude, 14190.72, 19900.0 / 65535.0);
    close(Tag::SensorHorizontalFieldOfView, 144.5713, 180.0 / 65535.0);
    close(Tag::SensorVerticalFieldOfView, 152.6436, 180.0 / 65535.0);
    close(
        Tag::SensorRelativeAzimuthAngle,
        160.71921143697557,
        360.0 / 4294967295.0,
    );
    close(
        Tag::SensorRelativeElevationAngle,
        -168.792324833941,
        360.0 / 4294967294.0,
    );
    close(Tag::SlantRange, 68590.98, 0.005); // spec value is rounded to 2 places
    close(Tag::TargetWidth, 722.8199, 10000.0 / 65535.0);
    close(
        Tag::FrameCenterLatitude,
        -10.5423886331461,
        180.0 / 4294967294.0,
    );
    close(
        Tag::FrameCenterLongitude,
        29.157890122923,
        360.0 / 4294967294.0,
    );
    close(Tag::FrameCenterElevation, 3216.037, 19900.0 / 65535.0);

    let sec = p.security().unwrap();
    assert_eq!(
        sec.classification(),
        Some(st0102::Classification::Unclassified)
    );
    assert_eq!(sec.text(st0102::Tag::ClassifyingCountry), Some("//USA"));
    assert!(sec.object_country_codes().unwrap().eq_str("USA"));
    assert_eq!(p.version(), Some(6));
}

/// klvdata does not check checksums (paretech/klvdata#58) and accepts this
/// MISB sample, whose checksum no longer matches its content. Every other
/// sample packet, including all 24 from the flight videos, verifies.
#[test]
fn sample_with_stale_checksum_is_flagged() {
    let bytes = data("DynamicConstantMISMMSPacketData.bin");
    assert_eq!(
        Packet::parse(&bytes),
        Err(misb::Error::Checksum {
            found: 0xAA43,
            computed: 0x3E1E
        })
    );
}

/// The flight samples give a plausible track: sensor over Alberta, frame
/// center a few km away, corners around the frame center.
#[test]
fn flight_track_is_plausible() {
    let bytes = data("day_flight.klv");
    let mut n = 0;
    for unit in klv::split(&bytes) {
        let p = Packet::from_unit(&unit).unwrap();
        let s = p.sensor_position().unwrap();
        let fc = p.frame_center().unwrap();
        assert!(
            (54.0..55.0).contains(&s.lat) && (-111.0..-110.0).contains(&s.lon),
            "{s:?}"
        );
        assert!((s.lat - fc.lat).abs() < 0.2 && (s.lon - fc.lon).abs() < 0.2);
        assert_eq!(
            p.target_location().map(|t| (t.lat, t.lon)),
            Some((fc.lat, fc.lon))
        );
        n += 1;
    }
    assert_eq!(n, 6);
}

/// Every single-bit flip of every sample packet is caught by the checksum or
/// the length checks.
#[test]
fn checksum_catches_bit_flips() {
    for file in FILES {
        let bytes = data(file);
        if file.starts_with("DynamicConstant") {
            continue;
        }
        let units: Vec<_> = klv::split(&bytes).map(|u| u.raw.to_vec()).collect();
        for raw in units {
            for i in 0..raw.len() {
                for bit in 0..8 {
                    let mut m = raw.clone();
                    m[i] ^= 1 << bit;
                    assert!(
                        Packet::parse(&m).map(|(_, n)| n == raw.len()).is_err() || m == raw,
                        "{file} byte {i} bit {bit}"
                    );
                }
            }
        }
    }
}

/// Decoding and re-encoding the sample packets through the builder gives
/// back the same bytes, item by item.
#[test]
fn re_encodes_sample_items() {
    use misb::st0601::{Builder, Kind};
    for file in FILES {
        let bytes = data(file);
        for unit in klv::split(&bytes) {
            let p = parse(file, &unit);
            if file.starts_with("DynamicConstant") {
                continue; // its checksum is stale, so the bytes cannot match
            }
            let mut b = Builder::new(p.timestamp().unwrap());
            for item in p.items().flatten() {
                if matches!(item.tag, 1 | 2) {
                    continue;
                }
                let tag = item.known_tag().unwrap();
                match (tag.info().kind, item.value()) {
                    (Kind::Linear { len, .. }, Value::Float(v))
                        if item.raw.len() == len as usize =>
                    {
                        b.float(tag, v).map(|_| ())
                    }
                    _ => b.raw(item.tag, item.raw).map(|_| ()),
                }
                .unwrap();
            }
            assert_eq!(b.finish(), unit.raw, "{file}");
        }
    }
}
