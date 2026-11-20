# Changelog

## 0.2.0 (2026-11-20)

Why a minor bump: `cargo install misb` now installs a binary whose job is
to fail builds, and its verdicts are a new contract. Most KLV in circulation
breaks ST 0601 or physics somewhere (see docs/conformance.md); a decoder
that reads it without saying so is how bad metadata reaches a map.

- `misb-check`: checks every ST 0601 packet in a `.ts`/`.mpg`/`.m2ts`, a
  raw `.klv` stream or a directory and exits 1 on: no KLV (or only EG 0104),
  missing Precision Time Stamp, Version Number or Checksum, Time Stamp not
  first, bad checksum, values in the wrong size, invalid text, ST 1201 values
  outside their mapping, IMAPB tags that disagree with the older tag for the
  same quantity, time stamps that go backwards or were never set, altitudes
  below the Dead Sea, frame centers above Everest, beyond the horizon or at
  the wrong slant range, sensors faster than 1,000 m/s, VMTI target counts
  that do not match. `--format text|json|github`, `--strict`.
- `action.yml`: the same check as a GitHub Action (`uses: Mattbusel/misb@v0.2.0`).
- `misb::conform` (`std`): `check_stream`, `Checker`, `Report`, `Finding`.
- `misb::ts` (`std`): takes the KLV data streams out of an MPEG transport
  stream (188, 192 and 204-byte packets) without ffmpeg.
- Fix: time tags other than the Precision Time Stamp decode at any length
  from 1 to 8 bytes. KWIVER sends Take-off Time (tag 131) in 7 bytes, which
  0.1.3 returned as `Invalid`. `Packet::timestamp` still requires tag 2 to be
  8 bytes, and so does the new `st0601::decode_tag`, which `Item::value`,
  `Packet::value` and the Python `decode_item` now use. Same change in
  `cpp/misb.hpp`.
- README: `pip install misb` was wrong (the package is not on PyPI); the
  wheels are on the GitHub release.

## 0.1.3 (2026-11-17)

- Python bindings in `python/` (PyO3, abi3 wheels for Python 3.10+, package
  `misb`): `parse` (checksum checked), `decode_item` (including IMAPB tags,
  the ST 0102 security set and the ST 0903 VMTI set with its targets),
  `Builder`, `split`, `tag_info`, `checksum`. The tests compare every value
  against the klvdata golden file, like the Rust tests. The Rust crate is
  unchanged.
- Typed: the package ships `py.typed` and stubs, and CI checks the stubs
  against the built module with `mypy.stubtest`.

## 0.1.2 (2026-11-17)

Documentation; no changes to the library code.

- New `docs/integration.md`: features, the API to depend on, MSRV,
  versioning, and where the KLV bytes come from in a video pipeline. Its
  example runs as a doctest.
- The README example now runs as a doctest too.

## 0.1.1 (2026-11-17)

- Match universal keys without byte 8 (the registry version byte), as SMPTE
  ST 336 says; some encoders send `0x0E` there instead of `0x01`. Applies to
  `st0601::Packet::parse`, `st0903::VmtiSet::parse` and the C++ header
  (`klv::same_key`).

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
