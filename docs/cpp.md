# C++ header

`cpp/misb.hpp` is a C++17 single-header port with the same tag table and
the same tests. Standard library only.

```cpp
#include "misb.hpp"

void on_klv(const uint8_t* data, size_t size) {
    for (const auto& unit : misb::klv::split(data, size)) {
        misb::Error err;
        auto p = misb::st0601::Packet::parse(unit.raw.data(), unit.raw.size(), &err);
        if (!p) { std::printf("bad packet: %s\n", misb::to_string(err)); continue; }
        if (auto s = p->sensor_position()) std::printf("sensor %.6f %.6f\n", s->lat, s->lon);
        for (const auto& item : p->items()) {
            if (auto n = p->number(item.tag))
                std::printf("%3u %s = %f\n", item.tag, misb::st0601::tag_name(item.tag), n.value);
        }
        if (auto v = p->vmti())
            for (const auto& t : v->targets())
                if (auto loc = t.location())
                    std::printf("target %llu at %.6f %.6f\n", (unsigned long long)t.id, loc->lat, loc->lon);
    }
}
```

Encoding:

```cpp
misb::st0601::Builder b(timestamp_us);
b.text(3, "TRAINING 4");
b.number(13, 38.8895);   // sensor latitude
b.number(14, -77.0353);  // sensor longitude
std::vector<uint8_t> packet = b.finish();  // key, length, items, version 19, checksum
```

With GStreamer, the KLV buffers from `tsdemux` (caps `meta/x-klv`) are
whole packets: pass the `gst_buffer_map` data to `Packet::parse`. With
FFmpeg, the `AVPacket` data of the `klv` data stream is the same.
