//! Takes the KLV data streams out of an MPEG-2 transport stream (`.ts`,
//! `.mpg`, `.m2ts`), the container STANAG 4609 video travels in, without
//! ffmpeg.
//!
//! Every elementary stream the Program Map Table declares as metadata
//! (`stream_type` 0x15) or private data (0x06, which is how most encoders
//! label KLV) is reassembled from its PES packets. The result is the same
//! byte stream that `ffmpeg -map 0:d -c copy -f data` writes, one per
//! stream, ready for [`crate::klv::split`] or
//! [`crate::conform::check_stream`].
//!
//! ```
//! # let file: Vec<u8> = Vec::new();
//! for s in misb::ts::klv_streams(&file) {
//!     println!("PID {:#x}: {} bytes of KLV", s.pid, s.data.len());
//! }
//! ```

use std::vec::Vec;

/// One data stream taken out of a transport stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataStream {
    /// The stream's PID.
    pub pid: u16,
    /// `stream_type` from the PMT (0x15 metadata, 0x06 private data), or 0
    /// when the file has no PMT.
    pub stream_type: u8,
    /// The PES payloads, concatenated.
    pub data: Vec<u8>,
    /// Number of PES packets.
    pub pes_packets: usize,
}

/// `true` if `buf` looks like an MPEG transport stream (188- or 192-byte
/// packets with the 0x47 sync byte).
pub fn is_transport_stream(buf: &[u8]) -> bool {
    layout(buf).is_some()
}

/// (offset of the first packet, packet size). 192-byte packets carry a
/// 4-byte time code before the sync byte; 204-byte packets carry 16 bytes
/// of Reed-Solomon parity after the 188.
fn layout(buf: &[u8]) -> Option<(usize, usize)> {
    for (size, pre) in [(188usize, 0usize), (192, 4), (204, 0)] {
        for start in 0..size {
            let full = buf.len().saturating_sub(start) / size;
            if full < 2 {
                break;
            }
            if (0..full.min(5)).all(|k| buf[start + k * size + pre] == 0x47) {
                return Some((start, size));
            }
        }
    }
    None
}

struct TsPacket<'a> {
    pid: u16,
    start: bool,
    payload: &'a [u8],
}

fn packets(buf: &[u8]) -> impl Iterator<Item = TsPacket<'_>> {
    let (start, size) = layout(buf).unwrap_or((0, 188));
    buf.get(start..)
        .unwrap_or(&[])
        .chunks_exact(size)
        .filter_map(move |c| {
            let p = if size == 192 { &c[4..] } else { &c[..188] };
            if p[0] != 0x47 {
                return None;
            }
            let pid = (u16::from(p[1] & 0x1f) << 8) | u16::from(p[2]);
            let start = p[1] & 0x40 != 0;
            let afc = (p[3] >> 4) & 3;
            if afc & 1 == 0 {
                return None;
            }
            let mut off = 4;
            if afc & 2 != 0 {
                off += 1 + p[4] as usize;
            }
            let payload = p.get(off..)?;
            Some(TsPacket {
                pid,
                start,
                payload,
            })
        })
}

/// A PSI section from a payload that starts one (pointer field first).
fn section(payload: &[u8]) -> Option<&[u8]> {
    let ptr = *payload.first()? as usize;
    let s = payload.get(1 + ptr..)?;
    let len = ((usize::from(*s.get(1)? & 0x0f)) << 8) | usize::from(*s.get(2)?);
    s.get(..3 + len)
}

/// The data streams in a transport stream, in PID order. Empty when the
/// input is not a transport stream or declares no metadata or private
/// data stream.
pub fn klv_streams(buf: &[u8]) -> Vec<DataStream> {
    if !is_transport_stream(buf) {
        return Vec::new();
    }
    // Pass 1: PAT, then the PMTs it names.
    let mut pmt_pids: Vec<u16> = Vec::new();
    let mut wanted: Vec<(u16, u8)> = Vec::new();
    let mut have_pmt = false;
    for p in packets(buf) {
        if !p.start {
            continue;
        }
        if p.pid == 0 {
            if let Some(s) = section(p.payload) {
                if s[0] == 0 && s.len() >= 12 {
                    for e in s[8..s.len() - 4].chunks_exact(4) {
                        let prog = u16::from_be_bytes([e[0], e[1]]);
                        let pid = u16::from_be_bytes([e[2] & 0x1f, e[3]]);
                        if prog != 0 && !pmt_pids.contains(&pid) {
                            pmt_pids.push(pid);
                        }
                    }
                }
            }
        } else if pmt_pids.contains(&p.pid) {
            let Some(s) = section(p.payload) else {
                continue;
            };
            if s[0] != 2 || s.len() < 16 {
                continue;
            }
            have_pmt = true;
            let pil = (usize::from(s[10] & 0x0f) << 8) | usize::from(s[11]);
            let end = s.len() - 4;
            let mut i = 12 + pil;
            while i + 5 <= end {
                let st = s[i];
                let pid = u16::from_be_bytes([s[i + 1] & 0x1f, s[i + 2]]);
                let eil = (usize::from(s[i + 3] & 0x0f) << 8) | usize::from(s[i + 4]);
                if (st == 0x15 || st == 0x06) && !wanted.iter().any(|w| w.0 == pid) {
                    wanted.push((pid, st));
                }
                i += 5 + eil;
            }
        }
    }
    // Pass 2: reassemble PES payloads.
    let mut out: Vec<DataStream> = wanted
        .iter()
        .map(|&(pid, stream_type)| DataStream {
            pid,
            stream_type,
            data: Vec::new(),
            pes_packets: 0,
        })
        .collect();
    let any = !have_pmt;
    for p in packets(buf) {
        if p.pid == 0 || p.pid == 0x1fff || pmt_pids.contains(&p.pid) {
            continue;
        }
        let idx = match out.iter().position(|s| s.pid == p.pid) {
            Some(i) => i,
            None if any => {
                out.push(DataStream {
                    pid: p.pid,
                    stream_type: 0,
                    data: Vec::new(),
                    pes_packets: 0,
                });
                out.len() - 1
            }
            None => continue,
        };
        let s = &mut out[idx];
        if p.start {
            let pl = p.payload;
            if pl.len() < 6 || pl[..3] != [0, 0, 1] {
                continue;
            }
            s.pes_packets += 1;
            let sid = pl[3];
            let no_ext = matches!(sid, 0xBC | 0xBE | 0xBF | 0xF0 | 0xF1 | 0xF2 | 0xF8 | 0xFF);
            let data = if no_ext {
                &pl[6..]
            } else {
                match pl.get(8).and_then(|&h| pl.get(9 + h as usize..)) {
                    Some(d) => d,
                    None => continue,
                }
            };
            s.data.extend_from_slice(data);
        } else if s.pes_packets > 0 {
            s.data.extend_from_slice(p.payload);
        }
    }
    out.retain(|s| !s.data.is_empty() || !any);
    out.sort_by_key(|s| s.pid);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn ts_packet(pid: u16, start: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![
            0x47,
            ((pid >> 8) as u8 & 0x1f) | if start { 0x40 } else { 0 },
            pid as u8,
            0x10,
        ];
        p.extend_from_slice(payload);
        // pad through an adaptation field would be correct; stuffing in the
        // payload is enough for these tests since KLV splitting skips it
        p.resize(188, 0xff);
        p
    }

    #[test]
    fn finds_klv_in_private_stream() {
        let klv = crate::st0601::Builder::new(1_700_000_000_000_000).finish();
        // PAT: program 1 -> PMT PID 0x100
        let pat = [
            0, 0x00, 0xB0, 13, 0, 1, 0xC1, 0, 0, 0, 1, 0xE1, 0x00, 0, 0, 0, 0,
        ];
        // PMT: PCR PID 0x101, one stream type 0x06 on PID 0x102
        let pmt = [
            0, 0x02, 0xB0, 18, 0, 1, 0xC1, 0, 0, 0xE1, 0x01, 0xF0, 0, 0x06, 0xE1, 0x02, 0xF0, 0, 0,
            0, 0, 0,
        ];
        let mut pes = vec![0, 0, 1, 0xBD, 0, 0, 0x84, 0x80, 5, 0x21, 0, 1, 0, 1];
        pes.extend_from_slice(&klv);
        let mut file = ts_packet(0, true, &pat);
        file.extend(ts_packet(0x100, true, &pmt));
        file.extend(ts_packet(0x102, true, &pes));
        file.extend(ts_packet(0x1fff, false, &[]));
        assert!(is_transport_stream(&file));
        let s = klv_streams(&file);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].pid, 0x102);
        assert_eq!(&s[0].data[..klv.len()], &klv[..]);
        let r = crate::conform::check_stream(&s[0].data);
        assert_eq!(r.packets, 1);
        assert!(r.passed(), "{:?}", r.findings);
    }

    #[test]
    fn not_a_transport_stream() {
        assert!(!is_transport_stream(b"hello world"));
        assert!(klv_streams(&[0u8; 1000]).is_empty());
    }
}
