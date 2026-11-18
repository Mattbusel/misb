"""Tests for the misb Python bindings, against the crate's sample data."""

import json
import math
import pathlib
import re

import pytest

import misb

DATA = pathlib.Path(__file__).resolve().parents[2] / "tests" / "data"
FILES = ["day_flight.klv", "night_flight_ir.klv", "DynamicConstantMISMMSPacketData.bin",
         "DynamicOnlyMISMMSPacketData.bin"]


def test_version_follows_pyproject():
    text = (pathlib.Path(__file__).parents[1] / "pyproject.toml").read_text()
    assert misb.__version__ == re.search(r'^version = "([^"]+)"', text, re.M).group(1)


def test_matches_klvdata_on_every_tag():
    """Every value paretech/klvdata decodes from the samples, decoded the same."""
    golden = [json.loads(line) for line in (DATA / "klvdata_golden.jsonl").read_text().splitlines()]
    g = iter(golden)
    compared = 0
    for name in FILES:
        for _key, _value, raw in misb.split((DATA / name).read_bytes()):
            items, used = misb.parse(raw, verify=False)
            assert used == len(raw)
            want = next(g)
            assert want["file"] == name
            by_tag = {}
            for item in items:
                by_tag.setdefault(item["tag"], item)
            for tag, expected in want["tags"].items():
                item = by_tag[int(tag)]
                got = item["value"]
                if item["kind"] == "security":
                    continue  # klvdata keeps the nested set as hex
                if int(tag) == 1:
                    assert f"{got:04x}" == expected
                elif isinstance(got, float):
                    assert math.isclose(got, expected, rel_tol=1e-9, abs_tol=1e-9), (name, tag)
                elif isinstance(got, bytes):
                    assert got.hex() == expected
                else:
                    assert got == expected, (name, tag)
                compared += 1
    assert next(g, None) is None
    assert compared > 500


def test_checksum_is_verified():
    bad = (DATA / "DynamicConstantMISMMSPacketData.bin").read_bytes()
    with pytest.raises(ValueError, match="checksum"):
        misb.parse(bad)
    items, _ = misb.parse(bad, verify=False)
    assert items[-1]["tag"] == 1


def test_key_version_byte_is_ignored():
    raw = misb.split((DATA / "day_flight.klv").read_bytes())[0][2]
    pkt = bytearray(raw)
    pkt[7] = 0x0E
    pkt[-2:] = misb.checksum(bytes(pkt[:-2])).to_bytes(2, "big")
    misb.parse(bytes(pkt))


def test_security_set_is_decoded():
    raw = (DATA / "DynamicConstantMISMMSPacketData.bin").read_bytes()
    items, _ = misb.parse(raw, verify=False)  # this MISB sample's checksum is stale
    sec = next(i for i in items if i["tag"] == 48)
    assert sec["status"] == "ok"
    assert sec["value"]["classification"] == "UNCLASSIFIED"
    assert sec["value"]["classifying_country"] == "//USA"
    assert sec["value"]["object_country_codes"] == "USA"
    assert sec["value"]["version"] == 10


def test_imapb_tags_are_decoded():
    assert misb.tag_info(113)["kind"] == "imapb"
    pkt = misb.Builder(1).float(113, 1234.5).float(120, 42.0).finish()
    items, _ = misb.parse(pkt)
    agl = next(i for i in items if i["tag"] == 113)
    full = next(i for i in items if i["tag"] == 120)
    assert agl["status"] == "ok" and abs(agl["value"] - 1234.5) < 0.1
    assert full["status"] == "ok" and abs(full["value"] - 42.0) < 0.01


def test_out_of_range_and_invalid():
    assert misb.decode_item(13, bytes.fromhex("80000000"))["status"] == "out_of_range"
    assert misb.decode_item(2, b"\x00")["status"] == "invalid"
    unknown = misb.decode_item(200, b"\x01")
    assert unknown["status"] == "raw" and unknown["value"] == b"\x01"


def test_vmti_set_is_decoded():
    for _key, value, _raw in misb.split((DATA / "vmti_rust.bin").read_bytes()):
        v = misb.decode_item(74, value)["value"]
        assert v["targets"], v
        assert all("id" in t for t in v["targets"])


def test_builder_round_trip():
    pkt = (misb.Builder(1_500_000_000_000_000)
           .text(3, "MISSION-42")
           .float(13, 60.1768229669783)
           .float(14, 128.42675904204452)
           .float(5, 159.9744)
           .finish())
    items, used = misb.parse(pkt)
    assert used == len(pkt)
    vals = {i["tag"]: i["value"] for i in items}
    assert vals[2] == 1_500_000_000_000_000
    assert vals[3] == "MISSION-42"
    assert abs(vals[13] - 60.1768229669783) < 1e-7
    assert vals[65] == 19
    assert items[-1]["tag"] == 1


def test_builder_rejects_out_of_range():
    with pytest.raises(ValueError):
        misb.Builder(0).float(13, 91.0)
    with pytest.raises(ValueError):
        misb.Builder(0).raw(1, b"\x00\x00")


def test_tag_info():
    info = misb.tag_info(13)
    assert info["name"] == "Sensor Latitude" and info["kind"] == "linear"
    assert misb.tag_info(500) is None
