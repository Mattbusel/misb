# misb: drone video without its KLV is just pictures

[![crates.io](https://img.shields.io/crates/v/misb.svg)](https://crates.io/crates/misb)
[![docs.rs](https://docs.rs/misb/badge.svg)](https://docs.rs/misb)
[![CI](https://github.com/Mattbusel/misb/actions/workflows/ci.yml/badge.svg)](https://github.com/Mattbusel/misb/actions/workflows/ci.yml)

Drone video carries KLV metadata: where the aircraft was, where the sensor pointed, what was detected in each frame. Without it the footage is pictures, not intelligence. And most of the KLV in circulation is broken in ways no player shows you: the video plays, the map pin lands, and the numbers behind it are wrong.

**`misb-check` fails the file instead.** It reads MISB ST 0601 out of a `.ts`/`.mpg` (no ffmpeg needed) or a raw `.klv` stream and exits 1 on missing KLV, missing mandatory tags (2, 65, 1), bad checksums, values sent in the wrong size, ST 1201 values that disagree with the tag they duplicate, time stamps that go backwards or were never set, and positions no aircraft or terrain can have. Every finding names the packet, its byte offset and the tag.

## What it found in the public samples (Nov 20 2026)

Every public ST 0601 sample we found in the KLV projects on GitHub and on samples.ffmpeg.org:

| File | Where it comes from | misb-check |
|---|---|---|
| `Day Flight.mpg`, `Night Flight IR.mpg` | [samples.ffmpeg.org](https://samples.ffmpeg.org/MPEG2/mpegts-klv/), the usual KLV test videos | **FAIL.** All 24 packets send Target Width (tag 22) in 4 bytes; ST 0601 maps it onto 2. Frame Center Elevation (tag 25) carries the sensor *elevation angle* in every packet (within 0.15): night packet 13 says the ground is at -36.7 m, where SRTM terrain is 600 m. The Slant Range (1,018 m) fits 600 m terrain; the sent elevation puts the ground 1,244 m below the sensor. 5 packets fail `SLANT-RANGE-MISMATCH`. |
| `DynamicConstantMISMMSPacketData.bin` | MISB example packet, test data in [klvdata](https://github.com/paretech/klvdata/tree/master/data), [misb.js](https://github.com/vidterra/misb.js/tree/main/test) and [QGIS FMV](https://github.com/All4Gis/QGISFMV/tree/master/code/tests/data) | **FAIL.** Checksum says `0xAA43`, the bytes sum to `0x3E1E`. The sensor looks at a frame center 11,535 km away. |
| `h264_klv.ts`, `h265_klv.ts`, `h265_tricky_klv.ts`, `mpeg2_klv.ts` | [KWIVER test data](https://github.com/Kitware/kwiver/tree/af3554f1fbe3cb30a29b77c2006aa78d6aa97902/test_data/videos) | **FAIL.** In each file's 30 full packets: Sensor Ellipsoid Height Extended (tag 104) says 23,456 m and Sensor Ellipsoid Height (tag 75) says 14,191 m, 4 to 7 more tag pairs disagree the same way, and the frame center is beyond the sensor's horizon. |
| `aphill_klv_10s.ts`, `aphill_short.ts` | KWIVER test data | **FAIL.** No ST 0601 at all, only the 2006 EG 0104 Predator universal set. |
| `foreman_cif_klv.ts`, `svt_testset_420_720p50_klved_4774.klv` | [jimcavoy/KlvDecoder](https://github.com/jimcavoy/KlvDecoder), [jimcavoy/klvp](https://github.com/jimcavoy/klvp) | PASS, with repeated time stamps as warnings. |

Then the parsers, fed the same 2,859 packets from 7 of those files:

| Parser | What happens |
|---|---|
| [klv-uas](https://crates.io/crates/klv-uas) 0.1.2 (6,102 downloads) | **Panics on 2,858 of 2,859 packets**: `todo!()` on the Security Local Set (tag 48, `klv_value.rs:189`) and a bitvec panic on the 4-byte Target Width. |
| [klvdata](https://pypi.org/project/klvdata/) (16,131 downloads last month) | Reads tags as one byte, so on KWIVER's ST 0601.19 packets it invents a tag 0, returns garbage for Platform Course Angle (tag 112), drops or garbles tags 128 to 143 and stops with `OverflowError` at packet 16 of 30. Halves Platform Sideslip Angle (Full) (tag 93: `_range = (-90, 90)`, ST 0601 says ±180). Never checks the checksum. |
| [marlin-klv](https://crates.io/crates/marlin-klv) 0.3.0 | Rejects all 30 KWIVER packets (it reads tags as one byte) and marks Target Width unparsable in the ffmpeg samples. |
| misb 0.1.3 (this crate) | Read Take-off Time (tag 131) as invalid when KWIVER sends it in 7 bytes. Fixed in 0.2.0. |

Details, commits and commands: [docs/conformance.md](docs/conformance.md).

## Check a file

```text
cargo install misb
misb-check flight.ts                  # or a directory, or a raw .klv stream
misb-check --format json flight.ts    # machine-readable
misb-check --strict flight.ts         # warnings fail too
```

Exit status: 0 pass, 1 fail, 2 unreadable input. In GitHub Actions:

```yaml
- uses: Mattbusel/misb@v0.3.0
  with:
    paths: test_data/videos    # files or directories, space separated
```

Findings show up as annotations on the run. In Rust, `misb::conform::check_stream` gives the same report, and `misb::ts` takes the KLV out of a transport stream (see below).

The decoder itself:

- ST 0601 UAS Datalink: all 143 tags, sensor and frame-center position, footprint corners
- ST 0903 VMTI: detections per frame, pixel boxes and ground positions, the Algorithm and Ontology series that say which detector found a target and what it is
- MPEG-TS in and out without ffmpeg: KLV with its PTS per packet, synchronous (ST 1402 AU cells) and asynchronous carriage, a push demuxer for live UDP feeds, a muxer for test files
- ST 0102 security markings, ST 1201 IMAPB
- Checks the checksum and every length. No panics on bad input, no dependencies, `no_std`
- Matches klvdata on every value of 26 public sample packets

## Install

```toml
[dependencies]
misb = "0.3"
```

C++17: copy [`cpp/misb.hpp`](cpp/misb.hpp).

Python: wheels are attached to each [GitHub release](https://github.com/Mattbusel/misb/releases) (`pip install misb-*.whl`, built from [`python/`](python/), see [python/DESCRIPTION.md](python/DESCRIPTION.md)).

## Use

```rust,no_run
use misb::{klv, st0601::Packet};

// ffmpeg -i in.ts -map 0:d -c copy -f data day_flight.klv
let stream = std::fs::read("day_flight.klv")?;
for unit in klv::split(&stream) {
    let Ok(p) = Packet::from_unit(&unit) else { continue };
    println!("{:?} sensor {:?} looking at {:?}", p.timestamp(), p.sensor_position(), p.frame_center());
}
# Ok::<(), std::io::Error>(())
```

### Straight from the video, with the frame time

`ts::klv_units` gives every KLV payload with its PID and 90 kHz PTS, so a
packet can be matched to the frame it describes. Synchronous KLV (stream
type 0x15, stream id 0xFC) comes wrapped in 5-byte metadata AU cell
headers; they are removed, so the bytes are plain KLV.

```rust,no_run
use misb::{klv, st0601::Packet, ts};

let file = std::fs::read("flight.ts")?;
for unit in ts::klv_units(&file) {
    for k in klv::split(&unit.data) {
        let Ok(p) = Packet::from_unit(&k) else { continue };
        println!("PTS {:?} s: frame center {:?}", unit.pts_seconds(), p.frame_center());
    }
}
# Ok::<(), std::io::Error>(())
```

For a live feed (UDP multicast, a pipe), `ts::Demuxer` takes datagrams of
any size, finds the sync byte again after loss, and returns each packet as
soon as its PES is complete:

```rust,no_run
let socket = std::net::UdpSocket::bind("0.0.0.0:5000")?;
let mut demux = misb::ts::Demuxer::new();
let mut buf = [0u8; 65_536];
loop {
    let n = socket.recv(&mut buf)?;
    demux.push(&buf[..n]);
    while let Some(unit) = demux.pop() {
        println!("PID {:#x} PTS {:?}: {} bytes of KLV", unit.pid, unit.pts, unit.data.len());
    }
}
# Ok::<(), std::io::Error>(())
```

`ts::Muxer` writes KLV into a transport stream, asynchronous (`KLVA`
registration) or synchronous (metadata descriptor, AU cells, PTS). ffmpeg
8.1 reads both back as `klv` and `ffmpeg -map 0:d -c copy -f data` returns
the original bytes, so it is a quick way to make test files for a ground
station:

```rust
use misb::{st0601::{Builder, Tag}, ts::Muxer};

let mut mux = Muxer::synchronous(0x1F1);
let mut file = mux.tables();
for frame in 0..30u64 {
    let mut b = Builder::new(1_700_000_000_000_000 + frame * 33_367);
    b.float(Tag::SensorLatitude, 38.8895)?.float(Tag::SensorLongitude, -77.0353)?;
    file.extend(mux.write(&b.finish(), Some(frame * 3_003)));
}
assert_eq!(misb::ts::klv_units(&file).len(), 30);
# Ok::<(), misb::Error>(())
```

## Docs

- [docs/conformance.md](docs/conformance.md): every check, the public samples and parsers it was run on
- [docs/format.md](docs/format.md): byte layouts and value mappings
- [docs/testing.md](docs/testing.md): reference data and how it is checked
- [docs/cpp.md](docs/cpp.md): the C++ header
- [python/DESCRIPTION.md](python/DESCRIPTION.md): the Python bindings (`parse`, `decode_item`, `Builder`, `split`)
- [docs/integration.md](docs/integration.md): using it inside another project (features, the API to depend on, MSRV, versioning)
- API: [docs.rs/misb](https://docs.rs/misb)

## License

MIT OR Apache-2.0.
