// Tests for misb.hpp against the same reference data as the Rust crate.
// Build and run from the repository root:
//   g++ -std=c++17 -O1 -Wall -Wextra -fsanitize=address,undefined -I cpp cpp/tests/test_misb.cpp -o test_misb && ./test_misb
#include "misb.hpp"

#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

static int failures = 0;
#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
            ++failures;                                                          \
        }                                                                        \
    } while (0)

static std::vector<uint8_t> read_file(const std::string& path) {
    std::ifstream f(path, std::ios::binary);
    if (!f) { std::fprintf(stderr, "cannot open %s\n", path.c_str()); std::exit(2); }
    return std::vector<uint8_t>((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
}

static std::string hex(misb::bytes_view b) {
    static const char* d = "0123456789abcdef";
    std::string s;
    for (uint8_t x : b) { s.push_back(d[x >> 4]); s.push_back(d[x & 15]); }
    return s;
}

static const std::string data_dir = "tests/data/";

// Every value klvdata decodes from the sample files (tests/data/klvdata_golden.tsv).
static void klvdata_golden() {
    std::map<std::string, std::vector<misb::klv::Unit>> units;
    std::map<std::string, std::vector<uint8_t>> files;
    std::ifstream tsv(data_dir + "klvdata_golden.tsv");
    CHECK(tsv.good());
    std::string line;
    int compared = 0;
    while (std::getline(tsv, line)) {
        std::istringstream ls(line);
        std::string file, idx, tag, kind, value;
        std::getline(ls, file, '\t'); std::getline(ls, idx, '\t'); std::getline(ls, tag, '\t');
        std::getline(ls, kind, '\t'); std::getline(ls, value);
        if (!files.count(file)) {
            files[file] = read_file(data_dir + file);
            units[file] = misb::klv::split(files[file].data(), files[file].size());
        }
        auto& u = units[file].at(std::stoul(idx));
        bool stale = file == "DynamicConstantMISMMSPacketData.bin";
        misb::Error err;
        auto p = stale ? misb::st0601::Packet::parse_unverified(u.raw.data(), u.raw.size(), &err)
                       : misb::st0601::Packet::parse(u.raw.data(), u.raw.size(), &err);
        CHECK(p.has_value());
        if (!p) continue;
        uint32_t t = static_cast<uint32_t>(std::stoul(tag));
        if (kind == "n") {
            double want = std::stod(value);
            auto got = p->value(t);
            CHECK(got.has_value());
            if (got && std::fabs(*got - want) > 1e-9 * std::max(1.0, std::fabs(want))) {
                std::fprintf(stderr, "%s #%s tag %u: %.12g vs klvdata %.12g\n", file.c_str(), idx.c_str(), t, *got, want);
                ++failures;
            }
        } else if (kind == "t") {
            CHECK(p->text(t) == std::optional<std::string_view>(value));
        } else {
            auto r = p->raw(t);
            CHECK(r && hex(*r) == value);
        }
        ++compared;
    }
    CHECK(compared > 500);
    std::printf("klvdata golden: %d values compared\n", compared);
}

static void sample_packet() {
    auto bytes = read_file(data_dir + "DynamicConstantMISMMSPacketData.bin");
    misb::Error err;
    CHECK(!misb::st0601::Packet::parse(bytes.data(), bytes.size(), &err));
    CHECK(err == misb::Error::checksum);  // stale checksum in the MISB sample
    auto p = misb::st0601::Packet::parse_unverified(bytes.data(), bytes.size());
    CHECK(p && p->timestamp() == 1231798102000000ull);
    CHECK(p->text(3) == std::optional<std::string_view>("Mission 12"));
    CHECK(std::fabs(*p->value(5) - 159.9744) < 360.0 / 65535);
    CHECK(std::fabs(*p->value(13) - 60.1768229669783) < 1e-7);
    auto sec = p->security();
    CHECK(sec && sec->classification() == 1);
    CHECK(sec->object_country_codes() == std::optional<std::string>("USA"));
    CHECK(sec->version() == 10ull);

    auto day = read_file(data_dir + "day_flight.klv");
    auto units = misb::klv::split(day.data(), day.size());
    CHECK(units.size() == 6);
    for (auto& u : units) {
        auto q = misb::st0601::Packet::parse(u.raw.data(), u.raw.size(), &err);
        CHECK(q.has_value());
        auto s = q->sensor_position();
        CHECK(s && s->lat > 54 && s->lat < 55 && s->lon > -111 && s->lon < -110);
    }
}

static void builder_round_trip() {
    misb::st0601::Builder b(1700000000000000ull);
    CHECK(b.text(3, "TRAINING 4") == misb::Error::none);
    CHECK(b.number(13, 38.8895) == misb::Error::none);
    CHECK(b.number(14, -77.0353) == misb::Error::none);
    CHECK(b.number(6, -0.4315251) == misb::Error::none);
    CHECK(b.number(103, 23456.24) == misb::Error::none);
    CHECK(b.number(6, 25.0) == misb::Error::out_of_range);
    auto bytes = b.finish();
    size_t used = 0;
    auto p = misb::st0601::Packet::parse(bytes.data(), bytes.size(), nullptr, &used);
    CHECK(p && used == bytes.size());
    CHECK(std::fabs(*p->value(13) - 38.8895) < 1e-7);
    CHECK(std::fabs(*p->value(103) - 23456.24) < 0.01);
    CHECK(p->version() == 19ull);
    // IMAPB examples from ST 0903 (0..180, 2 bytes)
    uint8_t fov[2];
    CHECK(misb::imapb::encode(0, 180, 10.0, fov, 2) && fov[0] == 0x05 && fov[1] == 0x00);
    CHECK(misb::imapb::encode(0, 180, 90.0, fov, 2) && fov[0] == 0x2D && fov[1] == 0x00);
    CHECK(!misb::imapb::decode(0, 180, misb::bytes_view((const uint8_t*)"\xC8\x00", 2)));
}

// Bytes produced by the Rust crate's st0903 builder (tests/data/vmti_rust.bin).
static void vmti_from_rust() {
    auto bytes = read_file(data_dir + "vmti_rust.bin");
    misb::Error err;
    auto v = misb::st0903::VmtiSet::parse(bytes.data(), bytes.size(), &err);
    CHECK(v.has_value());
    if (!v) return;
    CHECK(v->frame_width() == 1280ull);
    auto ts = v->targets();
    CHECK(ts.size() == 2);
    CHECK(ts[0].id == 7 && ts[0].confidence() == 87);
    auto loc = ts[0].location();
    CHECK(loc && std::fabs(loc->lat - 38.8895) < 1e-6 && std::fabs(loc->lon + 77.0353) < 1e-6);
    CHECK(misb::st0903::pixel_to_row_col(*ts[0].centroid_pixel(), 1280) == std::make_pair(uint64_t(360), uint64_t(640)));
    CHECK(ts[1].id == 300);
    auto off = ts[1].location_offset();
    CHECK(off && std::fabs(off->first - 0.001) < 1e-5);
}

static void random_input() {
    uint64_t s = 0x9E3779B97F4A7C15ull;
    auto next = [&] { s ^= s << 13; s ^= s >> 7; s ^= s << 17; return s; };
    auto day = read_file(data_dir + "day_flight.klv");
    for (int iter = 0; iter < 50000; ++iter) {
        std::vector<uint8_t> buf;
        if (iter % 2) {
            buf.assign(day.begin(), day.begin() + 163);
            for (int k = 0; k < 4; ++k) buf[next() % buf.size()] = static_cast<uint8_t>(next());
            if (next() % 3 == 0) buf.resize(1 + next() % buf.size());
        } else {
            buf.resize(next() % 300);
            for (auto& b : buf) b = static_cast<uint8_t>(next());
        }
        for (auto& u : misb::klv::split(buf.data(), buf.size())) {
            auto p = misb::st0601::Packet::parse_unverified(u.raw.data(), u.raw.size());
            if (!p) continue;
            for (auto& item : p->items()) (void)p->number(item.tag);
            (void)p->corners();
            if (auto v = p->vmti()) for (auto& t : v->targets()) (void)t.location();
            if (auto sc = p->security()) (void)sc->object_country_codes();
        }
        misb::st0903::VmtiSet v(misb::bytes_view(buf.data(), buf.size()));
        for (auto& t : v.targets()) { (void)t.location(); (void)t.location_offset(); }
    }
}

int main() {
    klvdata_golden();
    sample_packet();
    builder_round_trip();
    vmti_from_rust();
    random_input();
    if (failures) { std::fprintf(stderr, "%d failures\n", failures); return 1; }
    std::printf("all C++ tests passed\n");
    return 0;
}
