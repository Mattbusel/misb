# Conformance checks

`misb-check` (and `misb::conform::check_stream`) reads every MISB ST 0601
packet in a stream. Errors fail the file; warnings fail it only with
`--strict`.

| Code | Severity | What it means |
|---|---|---|
| `NO-KLV` | error | No ST 0601 packet in the file. |
| `ONLY-EG0104` | error | Only the 2006 EG 0104 Predator universal set; an ST 0601 decoder finds nothing. |
| `UNPARSEABLE` | error | A unit with the ST 0601 key whose length or framing is broken. |
| `CHECKSUM` | error | Tag 1 does not match the running sum of the packet. |
| `CHECKSUM-NOT-LAST` | error | The packet does not end with a 2-byte tag 1. |
| `TIMESTAMP-NOT-FIRST` | error | The first item is not the Precision Time Stamp (tag 2). |
| `MISSING-TIMESTAMP` | error | No tag 2. |
| `MISSING-VERSION` | error | No UAS Datalink LS Version Number (tag 65). |
| `BAD-ITEM` | error | An item inside the packet runs past its end. |
| `BAD-LENGTH` | error | A value in a size ST 0601 does not allow: tag 2 not 8 bytes, a linearly mapped tag in a size other than its own, tag 65 not 1 byte. |
| `BAD-TEXT` | error | A text tag that is not valid UTF-8. |
| `IMAPB-LENGTH` | error | An ST 1201 value of 0 or more than 8 bytes. |
| `IMAPB-RANGE` | error | An ST 1201 integer that maps above the tag's range. |
| `MAPPING-MISMATCH` | error | Two tags for the same quantity disagree by more than two steps of resolution: 104/75, 105/76, 103/38, 96/22, 90/6, 91/7, 92/50, 93/52. |
| `TIME-UNSET` | error | Tag 2 before 2000 or after 2100. |
| `TIME-BACKWARDS` | error | Tag 2 earlier than in the previous packet. |
| `ALTITUDE-IMPOSSIBLE` | error | Sensor altitude below -440 m MSL (the Dead Sea shore is -430 m) or ellipsoid height below -550 m. |
| `TERRAIN-IMPOSSIBLE` | error | Frame Center Elevation outside -440 to 8,850 m. |
| `BEYOND-HORIZON` | error | The frame center is further from the sensor than its horizon (`sqrt(2Rh + h^2)`, plus 10% and 1 km). |
| `SLANT-RANGE-MISMATCH` | error | Slant Range (tag 21) differs from the sensor-to-frame-center distance (WGS84, ECEF) by more than 20% and 250 m. |
| `SPEED-IMPOSSIBLE` | error | The sensor moved more than 1,000 m/s between two packets. |
| `VMTI-COUNT`, `VMTI-BAD-TARGET` | error | A nested ST 0903 set whose target count does not match its VTarget packs, or a malformed pack. |
| `DUPLICATE-TAG` | warning | A tag appears twice in one packet. |
| `UNKNOWN-TAG` | warning | Tag 0 or above 143. |
| `IMAPB-SPECIAL` | warning | An ST 1201 special value (top bits `11`). |
| `TIME-REPEAT` | warning | The same tag 2 as the previous packet. |
| `NULL-ISLAND` | warning | Sensor or frame center exactly at 0, 0. |

The mandatory items (tags 2, 65 and 1; 2 first, 1 last) are the ST 0601
ones. The physical limits are deliberately loose: a finding means the
numbers cannot describe a real flight.

## The public samples (Nov 20 2026)

| File | Source | Packets | Result |
|---|---|---|---|
| `Day Flight.mpg` | https://samples.ffmpeg.org/MPEG2/mpegts-klv/ | 6 | FAIL: `BAD-LENGTH` tag 22 x6 |
| `Night Flight IR.mpg` | same | 18 | FAIL: `BAD-LENGTH` tag 22 x18, `SLANT-RANGE-MISMATCH` x5 |
| `DynamicConstantMISMMSPacketData.bin` | paretech/klvdata `data/` @79028b4, also in vidterra/misb.js and All4Gis/QGISFMV | 1 | FAIL: `CHECKSUM`, `BEYOND-HORIZON`, `SLANT-RANGE-MISMATCH` |
| `DynamicOnlyMISMMSPacketData.bin` | same | 1 | FAIL: `BEYOND-HORIZON`, `SLANT-RANGE-MISMATCH` |
| `h264_klv.ts`, `h265_klv.ts`, `mpeg2_klv.ts` | Kitware/kwiver `test_data/videos` @af3554f | 30 each | FAIL: `MAPPING-MISMATCH` x237, `BEYOND-HORIZON` x30, `SLANT-RANGE-MISMATCH` x30 |
| `h265_tricky_klv.ts` | same | 90 | FAIL: the same 297 errors |
| `aphill_klv_10s.ts`, `aphill_short.ts` | same | 0 | FAIL: `ONLY-EG0104` |
| `h264_no_klv.ts`, `h264_audio.ts` | same | 0 | FAIL: `NO-KLV` (as their names say) |
| `0903.bin` | WestRidgeSystems/jmisb `api/src/test/resources` @ede21c1 | 0 | standalone ST 0903, no ST 0601: `NO-KLV` |
| `foreman_cif_klv.ts` | jimcavoy/KlvDecoder `sample/` @68334f7 | 300 | PASS, `TIME-REPEAT` x150 |
| `svt_testset_420_720p50_klved_4774.klv` | jimcavoy/klvp `sample/` @c570cb2 | 2,503 | PASS, `TIME-REPEAT` x1,993 |

What is behind the two ffmpeg samples: Frame Center Elevation (tag 25)
equals the Sensor Relative Elevation Angle (tag 19) to within 0.15 in all
24 packets, metres against degrees. In night packet 13 the sensor is at
1,207 m, tag 25 says the ground is at -36.7 m and SRTM (90 m) gives 600 m
at the frame center. With 600 m terrain the Ground Range (816 m) and the
height above ground give 1,017 m, against a Slant Range of 1,018 m. The
slant range is right and tag 25 is wrong; a tool that intersects the line
of sight with tag 25 puts the ground 637 m too low.

The KWIVER and MISB example packets are built from per-tag example values,
so tags that describe the same thing disagree, and the frame center is
thousands of kilometres from the sensor. A test that passes on them proves
the bytes parse, not that the geometry code works.

## The parsers, fed the same packets

Seven files (both ffmpeg samples, both MISB example packets, KWIVER
`h264_klv.ts`, `foreman_cif_klv.ts`, the SVT `.klv`): 2,859 ST 0601 packets.

| Parser | Result |
|---|---|
| klv-uas 0.1.2 (crates.io) | Panics on 2,858: `not yet implemented: Implement converting KLV bytes to Set` at `src/klv_value.rs:189` for every packet with a Security Local Set (tag 48), and `cannot load 16 bits from a 32-bit region` (bitvec) for the 4-byte Target Width. Only `DynamicOnlyMISMMSPacketData.bin` decodes. |
| marlin-klv 0.3.0 (crates.io) | Decodes the rest, rejects the 30 KWIVER packets (`LengthOverflow` x14, `Truncated` x16: tags read as one byte, ST 0601.19 tags 128 to 143 are two), marks the 4-byte Target Width unparsable. |
| klvdata @79028b4 (PyPI) | KWIVER `h264_klv.ts`: 16 of 30 packets, then `OverflowError` in `klvparser.py:66` (tags read as one byte, `setparser.py:39`; open PR #55 adds BER-OID tags). In those 16: a bogus tag 0, tag 112 (2 bytes) replaced by 111 bytes of later items, tags 128 to 143 dropped or wrong. Tag 93 decodes to -23.84 where the packet says -47.68 (`misb0601.py:1290`, `_range = (-90, 90)`; ST 0601 and jmisb use ±180). Tags 43 and 44 use 0..512 (`misb0601.py:644`, `:658`), ST 0601 and jmisb 0..510. Accepts the bad checksum (issue #58). |
| misb 0.1.3 | Returned tag 131 (Take-off Time, 7 bytes in KWIVER's files) as `Invalid`. Fixed in 0.2.0. |

Reproduce:

```text
misb-check --max 0 "Day Flight.mpg" "Night Flight IR.mpg" kwiver/test_data/videos/
```
