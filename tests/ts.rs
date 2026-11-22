//! Transport stream demux and mux: against a file ffmpeg wrote, and
//! round trips through our own muxer in both carriage modes.
use misb::klv;
use misb::st0601::{Builder, Packet, Tag};
use misb::ts::{klv_streams, klv_units, Demuxer, Muxer, PACKET_SIZE};

const DAY_FLIGHT: &[u8] = include_bytes!("data/day_flight.klv");
/// `ffmpeg -f lavfi -i testsrc -i <our sync KLV .ts> -map 0:v -map 1:d
/// -c:v mpeg2video -c:d copy -t 1.2`, ffmpeg 8.1.2: MPEG-2 video with PCR,
/// KLV re-muxed by ffmpeg as asynchronous private data with PTS.
const FFMPEG_TS: &[u8] = include_bytes!("data/ffmpeg_mpeg2_klv.ts");

fn day_flight_packets() -> Vec<&'static [u8]> {
    klv::split(DAY_FLIGHT).map(|u| u.raw).collect()
}

#[test]
fn reads_ffmpeg_file_with_video() {
    let want = day_flight_packets();
    let units = klv_units(FFMPEG_TS);
    assert_eq!(units.len(), want.len());
    for (u, w) in units.iter().zip(&want) {
        assert_eq!(u.pid, 0x101);
        assert_eq!(u.stream_type, 0x06);
        assert_eq!(&u.data[..], *w);
        Packet::parse(&u.data).unwrap();
    }
    // ffmpeg kept our 100 ms spacing.
    let pts: Vec<u64> = units.iter().map(|u| u.pts.unwrap()).collect();
    assert!(pts.windows(2).all(|w| w[1] - w[0] == 9_000), "{pts:?}");

    let streams = klv_streams(FFMPEG_TS);
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].data, DAY_FLIGHT);
    assert_eq!(streams[0].pes_packets, want.len());
    let report = misb::conform::check_stream(&streams[0].data);
    assert_eq!(report.packets, want.len());
}

fn mux(mut m: Muxer, packets: &[&[u8]]) -> Vec<u8> {
    let mut f = m.tables();
    for (i, p) in packets.iter().enumerate() {
        f.extend(m.write(p, Some(1_000 + i as u64 * 3_003)));
    }
    f
}

#[test]
fn round_trip_async_and_sync() {
    let want = day_flight_packets();
    for (m, st, sid) in [
        (Muxer::new(0x1F0), 0x06, 0xBD),
        (Muxer::synchronous(0x1F0), 0x15, 0xFC),
    ] {
        let f = mux(m, &want);
        assert_eq!(f.len() % PACKET_SIZE, 0);
        let units = klv_units(&f);
        assert_eq!(units.len(), want.len());
        for (i, (u, w)) in units.iter().zip(&want).enumerate() {
            assert_eq!((u.pid, u.stream_type, u.stream_id), (0x1F0, st, sid));
            assert_eq!(u.pts, Some(1_000 + i as u64 * 3_003));
            assert_eq!(&u.data[..], *w, "AU cell headers must not leak into KLV");
        }
        assert_eq!(klv_streams(&f)[0].data, DAY_FLIGHT);
    }
}

#[test]
fn payload_spanning_many_packets_and_cells() {
    // 70 kB: several hundred TS packets, a PES with length 0 (too long for
    // the 16-bit field) and, in sync mode, two AU cells.
    let mut b = Builder::new(5);
    let big = "x".repeat(127);
    for _ in 0..540 {
        b.text(Tag::MissionId, &big).unwrap();
    }
    let klv = b.finish();
    assert!(klv.len() > 0xFFFF);
    for m in [Muxer::new(0x44), Muxer::synchronous(0x44)] {
        let f = mux(m, &[&klv, &klv]);
        let units = klv_units(&f);
        assert_eq!(units.len(), 2);
        for u in &units {
            assert_eq!(u.data, klv);
        }
    }
}

#[test]
fn live_demuxer_matches_file_demux_for_any_chunking() {
    let want = day_flight_packets();
    for m in [Muxer::new(0x300), Muxer::synchronous(0x300)] {
        let f = mux(m, &want);
        let file = klv_units(&f);
        // Odd chunk sizes split packets and headers at every offset.
        for chunk in [1usize, 7, 187, 188, 189, 1316, 5000] {
            let mut d = Demuxer::new();
            let mut got = Vec::new();
            for c in f.chunks(chunk) {
                d.push(c);
                while let Some(u) = d.pop() {
                    got.push(u);
                }
            }
            d.flush();
            while let Some(u) = d.pop() {
                got.push(u);
            }
            assert_eq!(got, file, "chunk size {chunk}");
            assert_eq!(d.data_pids().collect::<Vec<_>>().len(), 1);
        }
    }
}

#[test]
fn live_demuxer_resyncs_after_garbage_and_loss() {
    let want = day_flight_packets();
    let mut m = Muxer::new(0x300);
    let mut feed = m.tables();
    feed.extend(m.write(want[0], Some(0)));
    feed.extend_from_slice(&[0x47, 1, 2, 3, 9, 0x47, 9]); // junk with fake sync bytes
    let second = m.write(want[1], Some(1));
    feed.extend_from_slice(&second[..100]); // a lost datagram cuts this PES
    feed.extend(m.write(want[2], Some(2)));
    feed.extend(m.write(want[3], Some(3)));
    let mut d = Demuxer::new();
    d.push(&feed);
    d.flush();
    let mut got = Vec::new();
    while let Some(u) = d.pop() {
        got.push(u);
    }
    assert_eq!(got[0].data, want[0]);
    assert_eq!(got.len(), 3, "the cut PES is dropped, the rest survive");
    assert_eq!((got[1].pts, &got[1].data[..]), (Some(2), want[2]));
    assert_eq!((got[2].pts, &got[2].data[..]), (Some(3), want[3]));
}

#[test]
fn demuxer_ignores_data_before_the_pmt() {
    let mut m = Muxer::new(0x300);
    let tables = m.tables();
    let early = m.write(b"\x06\x0e\x2b\x34\x00", Some(0));
    let mut d = Demuxer::new();
    d.push(&early);
    d.push(&tables);
    d.flush();
    assert!(d.pop().is_none());
}
