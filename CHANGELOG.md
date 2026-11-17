# Changelog

## 0.1.0 (2026-11-17)

- ST 0601.19 UAS Datalink Local Set: decode and encode all 143 tags (linear
  mappings, IMAPB, text, time, nested sets; packs passed through as bytes).
- ST 0102 Security Local Set: classification, country codes (UTF-16), text tags.
- ST 0903 VMTI Local Set: header, VTarget packs, location pack and offsets,
  standalone packets with their own key and checksum.
- ST 1201 IMAPB encode and decode with special-value handling.
- KLV stream splitter, BER length and BER-OID codecs, checksum.
- `cpp/misb.hpp`: C++17 single-header port, tested against the same data.
- Tested against paretech/klvdata on 26 public sample packets (641 values),
  bit-exact re-encoding of the flight samples, single-bit-flip detection,
  200,000 random and mutated inputs, cargo-fuzz target.
