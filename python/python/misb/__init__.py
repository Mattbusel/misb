"""MISB KLV metadata for drone video (STANAG 4609).

Python bindings for the Rust crate `misb`:

* ``parse(data, verify=True)``: a MISB ST 0601 UAS Datalink Local Set packet
  to a list of item dicts, with the key, length and checksum checked.
* ``decode_item(tag, raw)``: one ST 0601 item value, including the ST 1201
  IMAPB tags, the ST 0102 Security Local Set (tag 48) and the ST 0903 VMTI
  Local Set (tag 74).
* ``Builder(timestamp_us)``: encodes a packet, with the version and checksum.
* ``split(stream)``: KLV units in a raw data stream (ffmpeg ``-f data``).
* ``tag_info(tag)``, ``checksum(data)``.
"""

from ._misb import (
    UAS_LOCAL_SET_KEY,
    Builder,
    __version__,
    checksum,
    decode_item,
    parse,
    split,
    tag_info,
)

__all__ = [
    "Builder",
    "UAS_LOCAL_SET_KEY",
    "checksum",
    "decode_item",
    "parse",
    "split",
    "tag_info",
    "__version__",
]
