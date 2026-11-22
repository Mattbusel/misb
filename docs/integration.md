# Using misb inside another project

misb decodes and encodes the KLV metadata that rides next to the video in
STANAG 4609 streams. The project's existing MPEG-TS demuxer, GStreamer
`klvparse` / `appsink`, or `ffmpeg -map 0:d` can hand it the KLV bytes, or
`misb::ts` (with `std`) can take them out of the transport stream itself. A C++17 single header with the same
behaviour is in [`cpp/misb.hpp`](../cpp/misb.hpp) ([cpp.md](cpp.md)).

With `std`, `misb::ts::klv_units` takes each KLV payload out of an MPEG
transport stream with its PTS (`ts::Demuxer` does the same for a live feed,
`ts::Muxer` writes KLV back into a transport stream), and `misb::conform::check_stream` returns the
conformance report `misb-check` prints ([conformance.md](conformance.md)).

## Add it

```toml
[dependencies]
misb = "0.3"
```

| Feature | Default | Adds | Extra dependencies |
|---|---|---|---|
| `std` | yes | `std::error::Error` for `Error`; implies `alloc` | none |
| `alloc` | via `std` | the encoders (`st0601::Builder`, `st0102::Builder`, `st0903::Builder`) | none |

The crate has no dependencies. Decoding is `no_std`, zero-copy and does not
allocate; CI builds it for `thumbv7em-none-eabihf` and, with `alloc`, for
`wasm32-unknown-unknown`.

## The integration in five lines

Call this with each KLV payload your demuxer produces (one data PES, one
GStreamer buffer, or one unit from `misb::klv::split` over a raw stream):

```rust
use misb::st0601::Packet;

fn on_klv(buf: &[u8]) {
    let Ok((p, _len)) = Packet::parse(buf) else { return }; // checksum and lengths checked
    println!("{:?} us: sensor {:?}, looking at {:?}, footprint {:?}", p.timestamp(), p.sensor_position(), p.frame_center(), p.corners());
    let detections = p.vmti().map(|v| v.targets().count()).unwrap_or(0); // ST 0903 VMTI
    println!("{detections} detections");
}
```

`Packet::parse_unverified` skips the checksum for sources known to send a
wrong one. Every tag is also reachable by number through `Packet::items`
and `Packet::value`, with names and units from `st0601::tag_info`.

## The surface to depend on

| Item | What it is |
|---|---|
| `klv::split`, `klv::same_key` | a raw KLV byte stream to units, key matching that ignores the version byte |
| `st0601::Packet` (`parse`, `items`, `value`, `float`, `text`, `timestamp`, `sensor_position`, `frame_center`, `corners`, `vmti`, `security`) | UAS Datalink Local Set, all 143 tags |
| `st0601::Builder`, `st0601::Tag`, `st0601::Value` | encoding, tag names, decoded values |
| `st0903::VmtiSet`, `st0903::VTarget`, `st0903::Algorithm`, `st0903::Ontology` | VMTI detections: pixel boxes, confidence, ground positions, the algorithm and ontology class behind each |
| `ts::klv_units`, `ts::klv_streams`, `ts::Demuxer`, `ts::Muxer` (`std`) | KLV out of and into MPEG transport streams, with PTS |
| `st0102::SecuritySet` | security markings |
| `imapb` | ST 1201 float mapping |
| `GeoPoint`, `Error` | WGS84 point, one error type |

No function panics on input bytes. That is checked with 200,000 random and
mutated inputs in the tests and a `cargo-fuzz` target, and the C++ header
runs the same data under ASan and UBSan in CI.

## Compatibility

- MSRV: Rust 1.70, checked in CI. Edition 2021.
- License: MIT OR Apache-2.0.
- `#![forbid(unsafe_code)]`. CI runs on Linux, Windows and macOS.
- Versioning: while the crate is 0.x, a breaking change bumps the minor
  version (0.1 to 0.2) and is listed in [CHANGELOG.md](../CHANGELOG.md).
- Values match paretech/klvdata on 26 public sample packets
  ([testing.md](testing.md)).
- Scope: metadata for situational awareness and interoperability. Weapon
  tags (ST 0601 tags 60, 61, 62 and 140) are passed through as raw bytes and
  never interpreted.

Maintained by Matt Busel ([github.com/Mattbusel](https://github.com/Mattbusel)).
`uas-cot` builds on it to put KLV platforms and sensor footprints on TAK
maps. Issues and pull requests:
[github.com/Mattbusel/misb](https://github.com/Mattbusel/misb/issues).
