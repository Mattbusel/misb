# How it is tested

## Reference data (`tests/data`)

| File | Source |
|---|---|
| `day_flight.klv` (6 packets) | KLV stream of the public "Day Flight.mpg" sample, https://samples.ffmpeg.org/MPEG2/mpegts-klv/ |
| `night_flight_ir.klv` (18 packets) | KLV stream of "Night Flight IR.mpg" from the same place |
| `DynamicConstantMISMMSPacketData.bin`, `DynamicOnlyMISMMSPacketData.bin` | MISB sample packets shipped with [paretech/klvdata](https://github.com/paretech/klvdata) (MIT) |
| `klvdata_golden.jsonl`, `klvdata_golden.tsv` | every value klvdata decodes from those 26 packets |
| `vmti_rust.bin` | a VMTI packet from the Rust encoder, read by the C++ tests |

Extract a KLV stream from a video:

```
ffmpeg -i "Day Flight.mpg" -map 0:d -c copy -f data day_flight.klv
```

Regenerate the golden files with a klvdata checkout:

```
python tools/golden.py ../klvdata tests/data/day_flight.klv tests/data/night_flight_ir.klv \
  tests/data/DynamicConstantMISMMSPacketData.bin tests/data/DynamicOnlyMISMMSPacketData.bin > tests/data/klvdata_golden.jsonl
python tools/golden.py --tsv tests/data/klvdata_golden.jsonl > tests/data/klvdata_golden.tsv
```

## What the tests check

- `tests/reference.rs`: all 641 values klvdata reads from the 26 packets
  match (floats within 1e-9 relative). The MISB example values ("Mission
  12", "Predator") decode within one quantisation step.
- Every checksum in the flight samples verifies.
  `DynamicConstantMISMMSPacketData.bin` does not: its stored checksum is
  `0xAA43`, the content sums to `0x3E1E`. klvdata does not check checksums
  (klvdata issue #58) and accepts it.
- Every single-bit flip of every sample packet is rejected.
- Decoding the flight packets and encoding the values again with
  `st0601::Builder` gives back the same bytes.
- `tests/robustness.rs`: 100,000 random buffers and 100,000 mutated sample
  packets through every decoder without a panic.
- `fuzz/`: `cargo fuzz run parse` (CI runs it for two minutes per push).
- `cpp/tests/test_misb.cpp`: the C++ header against the same golden file,
  the Rust-encoded VMTI packet and 50,000 random inputs, under ASan and
  UBSan (g++, clang++) and MSVC `/W4 /WX`.
