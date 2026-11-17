"""Decode KLV files with paretech/klvdata and write one JSON line per packet: {tag: value}."""
import sys, json, datetime
if sys.argv[1] == "--tsv":
    counts = {}
    for line in open(sys.argv[2], encoding="utf-8"):
        rec = json.loads(line)
        f = rec["file"]
        i = counts.get(f, 0)
        counts[f] = i + 1
        for tag, v in rec["tags"].items():
            if isinstance(v, dict) or tag == "1":
                continue
            kind = "n" if isinstance(v, (int, float)) else ("t" if tag in ("3", "4", "10", "11", "12", "59", "70") else "h")
            val = repr(float(v)) if kind == "n" else v
            sys.stdout.write("\t".join([f, str(i), tag, kind, str(val)]) + "\n")
    sys.exit(0)

sys.path.insert(0, sys.argv[1])  # path to klvdata checkout
import klvdata
from klvdata.misb0601 import UASLocalMetadataSet

def val(el):
    v = getattr(el, "value", None)
    if hasattr(v, "value"):
        v = v.value
    if isinstance(v, datetime.datetime):
        return int(v.timestamp() * 1_000_000) if v.tzinfo else int(v.replace(tzinfo=datetime.timezone.utc).timestamp() * 1_000_000)
    if isinstance(v, (bytes, bytearray)):
        return v.hex()
    if isinstance(v, (int, float, str)) or v is None:
        return v
    return str(v)

for path in sys.argv[2:]:
    data = open(path, "rb").read()
    for packet in klvdata.StreamParser(data):
        if not isinstance(packet, UASLocalMetadataSet):
            continue
        out = {}
        for key, el in packet.items.items():
            tag = int.from_bytes(key, "big")
            if tag == 48:
                sub = {int.from_bytes(k, "big"): val(e) for k, e in el.items.items()} if hasattr(el, "items") else val(el)
                out["48"] = sub
            else:
                out[str(tag)] = val(el)
        print(json.dumps({"file": path.replace("\\", "/").split("/")[-1], "tags": out}))

# Usage: python tools/golden.py <klvdata checkout> tests/data/day_flight.klv ... > tests/data/klvdata_golden.jsonl
# Then:  python tools/golden.py --tsv tests/data/klvdata_golden.jsonl > tests/data/klvdata_golden.tsv  (read by the C++ tests)
