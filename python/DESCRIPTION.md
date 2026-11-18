# misb

MISB KLV metadata for drone video (STANAG 4609) in Python, backed by the
Rust crate [`misb`](https://crates.io/crates/misb). It decodes and encodes
the MISB ST 0601.19 UAS Datalink Local Set, including the ST 1201 IMAPB
tags, the ST 0102 Security Local Set and the ST 0903 VMTI Local Set, and it
checks the checksum on every packet.

```python
import misb

# The KLV data stream of an MPEG-TS file:
#   ffmpeg -i flight.ts -map 0:d -c copy -f data flight.klv
stream = open("flight.klv", "rb").read()
for key, value, raw in misb.split(stream):
    items, used = misb.parse(raw)          # ValueError on a bad checksum
    for item in items:
        print(item["tag"], item["name"], item["value"], item["units"])

# One item value
misb.decode_item(13, bytes.fromhex("5595B66D"))["value"]   # 60.1768229669783

# Encoding: values outside a tag's range raise instead of being clamped
packet = (misb.Builder(timestamp_us=1_700_000_000_000_000)
          .text(3, "MISSION 12")
          .float(13, 60.1768).float(14, 128.4268)
          .float(113, 1234.5)        # Altitude AGL, IMAPB
          .finish())                 # version (tag 65) and checksum added
```

Each item is a dict with `tag`, `name`, `units`, `kind`, `raw` and
`status` / `value`:

| status | value |
|---|---|
| `"ok"` | float, int, str, or a dict for tag 48 (ST 0102) and tag 74 (ST 0903, with its targets) |
| `"out_of_range"` | `None`: the sender's out-of-range marker |
| `"invalid"` | `None`: wrong length, or not text |
| `"raw"` | `bytes`: packs, other nested sets and unknown tags |

The Rust crate is tested against paretech/klvdata on 26 public sample
packets (641 values), bit-exact re-encoding of the flight samples,
single-bit-flip detection and fuzzing; these bindings run the same klvdata
comparison. The same decoder is available as a C++17 single header.

Licensed under MIT or Apache-2.0, at your option.
