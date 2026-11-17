//! Random and mutated input through every decoder: nothing may panic and
//! nothing may read outside the input. (`fuzz/` has the cargo-fuzz targets
//! for longer runs.)

use misb::st0601::{Packet, Value};
use misb::{klv, st0903};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

fn walk(p: &Packet<'_>) -> usize {
    let mut n = 0;
    for item in p.items().flatten() {
        n += 1;
        match item.value() {
            Value::Security(s) => {
                let _ = (
                    s.classification(),
                    s.version(),
                    s.text(misb::st0102::Tag::ClassifyingCountry),
                );
                if let Some(c) = s.object_country_codes() {
                    n += c.chars().count();
                }
            }
            Value::Vmti(v) => n += walk_vmti(&v),
            v => {
                let _ = v.as_f64();
            }
        }
    }
    let _ = (
        p.corners(),
        p.sensor_position(),
        p.frame_center(),
        p.timestamp(),
        p.platform_pitch(),
        p.sensor_hae(),
    );
    n
}

fn walk_vmti(v: &st0903::VmtiSet<'_>) -> usize {
    let _ = (
        v.horizontal_fov(),
        v.frame_width(),
        v.system_name(),
        v.timestamp(),
    );
    let mut n = 0;
    for t in v.targets().flatten() {
        let _ = (
            t.location(),
            t.ground_position(None),
            t.centroid_row_col(v.frame_width().unwrap_or(0)),
            t.bounding_box(),
        );
        n += t.items().count();
    }
    n
}

fn samples() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for f in [
        "day_flight.klv",
        "night_flight_ir.klv",
        "DynamicOnlyMISMMSPacketData.bin",
    ] {
        let bytes =
            std::fs::read(format!("{}/tests/data/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        out.extend(klv::split(&bytes).map(|u| u.raw.to_vec()));
    }
    let mut v = st0903::Builder::new(1);
    v.frame_size(100, 100).unwrap();
    let mut t = st0903::TargetBuilder::new(1);
    t.location(1.0, 2.0, 3.0)
        .unwrap()
        .centroid_pixel(50)
        .unwrap();
    v.target(&t).unwrap();
    let mut b = misb::st0601::Builder::new(1);
    b.vmti(&v).unwrap();
    out.push(b.finish());
    out
}

#[test]
fn random_bytes() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..100_000 {
        let len = (rng.next() % 300) as usize;
        let mut buf: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        if len > 17 && rng.next() % 2 == 0 {
            buf[..16].copy_from_slice(&misb::st0601::KEY);
        }
        if let Ok((p, n)) = Packet::parse_unverified(&buf) {
            assert!(n <= buf.len());
            walk(&p);
        }
        walk(&Packet::from_body(&buf));
        walk_vmti(&st0903::VmtiSet::new(&buf));
        for u in klv::split(&buf) {
            assert!(u.raw.len() >= 17 && u.raw.len() <= buf.len());
        }
    }
}

#[test]
fn mutated_samples() {
    let mut rng = Rng(42);
    let samples = samples();
    for _ in 0..100_000 {
        let mut buf = samples[(rng.next() % samples.len() as u64) as usize].clone();
        for _ in 0..1 + rng.next() % 6 {
            let i = (rng.next() % buf.len() as u64) as usize;
            match rng.next() % 3 {
                0 => buf[i] = rng.byte(),
                1 => buf.truncate(i.max(1)),
                _ => buf.insert(i, rng.byte()),
            }
        }
        if let Ok((p, _)) = Packet::parse_unverified(&buf) {
            walk(&p);
        }
        let _ = Packet::parse(&buf);
    }
}
