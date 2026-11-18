# misb

[![crates.io](https://img.shields.io/crates/v/misb.svg)](https://crates.io/crates/misb)
[![docs.rs](https://docs.rs/misb/badge.svg)](https://docs.rs/misb)
[![CI](https://github.com/Mattbusel/misb/actions/workflows/ci.yml/badge.svg)](https://github.com/Mattbusel/misb/actions/workflows/ci.yml)

Read and write the KLV metadata in drone video (STANAG 4609), in Rust, Python and one C++ header.

- ST 0601 UAS Datalink: all 143 tags, sensor and frame-center position, footprint corners
- ST 0903 VMTI: detections per frame, pixel boxes and ground positions
- ST 0102 security markings, ST 1201 IMAPB
- Checks the checksum and every length. No panics on bad input, no dependencies, `no_std`
- Matches klvdata on every value of 26 public sample packets

## Install

```toml
[dependencies]
misb = "0.1"
```

C++17: copy [`cpp/misb.hpp`](cpp/misb.hpp).

Python: `pip install misb` (built from [`python/`](python/), see [python/DESCRIPTION.md](python/DESCRIPTION.md)).

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

## Docs

- [docs/format.md](docs/format.md): byte layouts and value mappings
- [docs/testing.md](docs/testing.md): reference data and how it is checked
- [docs/cpp.md](docs/cpp.md): the C++ header
- [python/DESCRIPTION.md](python/DESCRIPTION.md): the Python bindings (`parse`, `decode_item`, `Builder`, `split`)
- [docs/integration.md](docs/integration.md): using it inside another project (features, the API to depend on, MSRV, versioning)
- API: [docs.rs/misb](https://docs.rs/misb)

## License

MIT OR Apache-2.0.
