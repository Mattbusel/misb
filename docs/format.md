# Format notes

## Packet

```
06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00   16-byte key (ST 0601)
81 91                                             BER length (here 145)
02 08 00 04 6C 8E 20 03 83 85                     tag 2, length 8, Precision Time Stamp
41 01 01                                          tag 65, length 1, version 1
...
01 02 1C 5F                                       tag 1, length 2, checksum
```

- Tags are BER-OID: one byte up to 127, then `81 00` = 128 ... `81 0F` = 143.
- Lengths are BER: one byte up to 127, else `81 xx`, `82 xx xx`. Up to `84`
  (4 GiB) is accepted; `80` (indefinite) is rejected.
- The checksum is a 16-bit sum over the key through the checksum length
  byte, adding even-offset bytes as the high byte:
  `sum += byte << (8 * ((i + 1) % 2))`.

## Value mappings

| Kind | Decode | Example |
|---|---|---|
| Unsigned linear, `n` bytes | `raw * (max - min) / (2^(8n) - 1) + min` | tag 15 altitude: `raw * 19900 / 65535 - 900` |
| Signed linear, `n` bytes | `raw * (max - min) / (2^(8n) - 2)` | tag 13 latitude: `raw * 180 / 4294967294` |
| Signed "out of range" | `0x8000`, `0x80000000` decode to `Value::OutOfRange` | |
| IMAPB (ST 1201) | `sR * (y - zOffset) + min`, `sR = 2^(bPow - dPow)` | tag 103: `-900..40000` |
| Time | unsigned 8 bytes, microseconds since 1970 UTC | tag 2 |

Mapped tags sent with a non-standard length (older encoders write tag 22 in
four bytes) are read as integers on the standard scale, as klvdata does.

IMAPB integers whose two top bits are set are special values (infinity,
NaN, below or above range) and decode to `None` / `Value::OutOfRange`.

## Footprint

`Packet::corners()` returns the four ground corners. It uses tags 82 to 89
(full precision) when all eight are present, else adds the offset tags 26 to
33 (each within +-0.075 degrees) to the frame center (tags 23, 24).

## VMTI (ST 0903)

```
VTarget Series (tag 101) = { BER length, VTarget Pack }*
VTarget Pack            = Target ID (BER-OID), then local set items
```

- Pixel numbers are 1-based and row-major: `pixel = column + (row - 1) * width`.
- Target Location (tag 17) is `lat IMAPB(-90, 90, 4) | lon IMAPB(-180, 180, 4) | height IMAPB(-900, 19000, 2)`,
  optionally followed by standard deviations and correlations (not decoded).
- Offsets (tags 10, 11) are IMAPB(-19.2, 19.2, 3) degrees from the frame center.

## Not interpreted

ST 0601 tags 60, 61, 62 and 140 describe weapons. They are returned as raw
bytes and never decoded. Packs and nested sets of other standards (ST 0806
RVT, ST 1206 SAR, ST 1002, ST 1601, ST 1602, ST 1607, waypoint and payload
lists) are also returned as bytes.

## Sources

All of these are public MISB documents (Distribution A, approved for public
release) published by the NGA Motion Imagery Standards Board: ST 0601.19,
ST 0102.12, ST 0903.6, ST 1201.5, ST 0107.
