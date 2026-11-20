//! Pins the conformance verdicts on the public samples, and the CLI's exit codes.
#![cfg(feature = "std")]

use misb::conform::{check_stream, Report};
use std::process::Command;

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn count(r: &Report, code: &str) -> usize {
    r.findings.iter().filter(|f| f.code == code).count()
}

#[test]
fn day_flight_sends_target_width_in_four_bytes() {
    let r = check_stream(&read("day_flight.klv"));
    assert_eq!(r.packets, 6);
    assert_eq!(count(&r, "BAD-LENGTH"), 6);
    assert!(r.findings.iter().all(|f| f.tag == Some(22)));
    assert_eq!(r.errors(), 6);
}

#[test]
fn night_flight_frame_center_is_below_the_slant_range() {
    let r = check_stream(&read("night_flight_ir.klv"));
    assert_eq!(r.packets, 18);
    assert_eq!(count(&r, "BAD-LENGTH"), 18);
    // Frame Center Elevation (tag 25) carries the sensor elevation angle,
    // so the steep-look packets put the ground over 1,200 m below a sensor
    // whose Slant Range is 640 to 1,018 m.
    assert_eq!(count(&r, "SLANT-RANGE-MISMATCH"), 5);
    assert_eq!(r.errors(), 23);
}

#[test]
fn misb_example_packets() {
    let r = check_stream(&read("DynamicConstantMISMMSPacketData.bin"));
    assert_eq!(count(&r, "CHECKSUM"), 1);
    assert_eq!(count(&r, "BEYOND-HORIZON"), 1);
    let r = check_stream(&read("DynamicOnlyMISMMSPacketData.bin"));
    assert_eq!(count(&r, "CHECKSUM"), 0);
    assert_eq!(count(&r, "BEYOND-HORIZON"), 1);
}

#[test]
fn every_single_byte_corruption_is_caught() {
    let clean = read("day_flight.klv");
    let first = &clean[..0xa3];
    for i in 17..first.len() {
        let mut p = first.to_vec();
        p[i] ^= 0x40;
        let r = check_stream(&p);
        assert!(!r.passed(), "byte {i}");
    }
}

fn cli(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_misb-check"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    )
}

#[test]
fn cli_exit_codes() {
    let (code, out) = cli(&["tests/data/day_flight.klv"]);
    assert_eq!(code, 1);
    assert!(out.contains("FAIL") && out.contains("BAD-LENGTH"), "{out}");

    let (code, out) = cli(&["--format", "json", "tests/data/vmti_rust.bin"]);
    assert_eq!(code, 1);
    assert!(out.contains("\"NO-KLV\""), "{out}");

    let (code, out) = cli(&["--format", "github", "tests/data/night_flight_ir.klv"]);
    assert_eq!(code, 1);
    assert!(out.contains("::error file="), "{out}");

    let dir = std::env::temp_dir().join(format!("misb-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut b = misb::st0601::Builder::new(1_700_000_000_000_000);
    b.float(misb::st0601::Tag::SensorLatitude, 38.9).unwrap();
    let good = dir.join("good.klv");
    std::fs::write(&good, b.finish()).unwrap();
    let (code, out) = cli(&[good.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("PASS"));

    let (code, _) = cli(&["--no-such-flag"]);
    assert_eq!(code, 2);
    let (code, _) = cli(&[dir.join("missing.ts").to_str().unwrap()]);
    assert_eq!(code, 2);
    std::fs::remove_dir_all(&dir).ok();
}
