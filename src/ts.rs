//! Takes KLV out of an MPEG-2 transport stream (`.ts`, `.mpg`, `.m2ts`),
//! the container STANAG 4609 video travels in, and puts it back in,
//! without ffmpeg.
//!
//! Every elementary stream the Program Map Table declares as metadata
//! (`stream_type` 0x15) or private data (0x06, which is how most encoders
//! label KLV) is reassembled from its PES packets. Synchronous KLV (MISB
//! ST 1402, `stream_id` 0xFC) wraps each payload in ISO 13818-1 metadata
//! Access Unit cells; their 5-byte headers are removed so the result is
//! plain KLV.
//!
//! - [`klv_units`]: every PES payload with its PID and 90 kHz PTS, in file
//!   order. Use the PTS to line a packet up with its video frame.
//! - [`klv_streams`]: the same payloads concatenated per PID, the byte
//!   stream that `ffmpeg -map 0:d -c copy -f data` writes, ready for
//!   [`crate::klv::split`] or [`crate::conform::check_stream`].
//! - [`Demuxer`]: the same for a live feed (UDP or RTP multicast, a pipe)
//!   that arrives in chunks of any size.
//! - [`Muxer`]: writes KLV into a transport stream, asynchronous (private
//!   data, `KLVA` registration) or synchronous (metadata AU cells), for
//!   test fixtures, simulators and re-muxing.
//!
//! ```
//! use misb::st0601::{Builder, Packet};
//! use misb::ts::{klv_units, Muxer};
//!
//! let klv = Builder::new(1_700_000_000_000_000).finish();
//! let mut mux = Muxer::new(0x102);
//! let mut file = mux.tables();
//! file.extend(mux.write(&klv, Some(90_000)));
//!
//! for unit in klv_units(&file) {
//!     assert_eq!(unit.pts, Some(90_000));
//!     let (packet, _) = Packet::parse(&unit.data).unwrap();
//!     assert_eq!(packet.timestamp(), Some(1_700_000_000_000_000));
//! }
//! ```

use std::collections::VecDeque;
use std::vec;
use std::vec::Vec;

use crate::klv::UL_PREFIX;

/// Size of a transport stream packet.
pub const PACKET_SIZE: usize = 188;

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

/// The payload of one PES packet from a metadata or private data stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KlvUnit {
    /// The stream's PID.
    pub pid: u16,
    /// `stream_type` from the PMT (0x15 metadata, 0x06 private data), or 0
    /// when the file has no PMT.
    pub stream_type: u8,
    /// `stream_id` from the PES header (0xFC synchronous metadata, 0xBD
    /// private stream 1 for asynchronous KLV).
    pub stream_id: u8,
    /// Presentation time stamp in 90 kHz ticks (33 bits), if the PES
    /// header has one. Synchronous KLV always does; asynchronous KLV may.
    pub pts: Option<u64>,
    /// The payload: KLV bytes, with metadata AU cell headers removed.
    pub data: Vec<u8>,
}

impl KlvUnit {
    /// [`KlvUnit::pts`] in seconds.
    pub fn pts_seconds(&self) -> Option<f64> {
        self.pts.map(|p| p as f64 / 90_000.0)
    }
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

/// Parses one 188-byte packet. `None` for a bad sync byte, a packet
/// without payload or an adaptation field longer than the packet.
fn parse_packet(p: &[u8]) -> Option<TsPacket<'_>> {
    if p.len() < PACKET_SIZE || p[0] != 0x47 {
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
    let payload = p.get(off..PACKET_SIZE)?;
    Some(TsPacket {
        pid,
        start,
        payload,
    })
}

fn packets(buf: &[u8]) -> impl Iterator<Item = TsPacket<'_>> {
    let (start, size) = layout(buf).unwrap_or((0, 188));
    buf.get(start..)
        .unwrap_or(&[])
        .chunks_exact(size)
        .filter_map(move |c| parse_packet(if size == 192 { &c[4..] } else { &c[..188] }))
}

/// A PSI section from a payload that starts one (pointer field first).
fn section(payload: &[u8]) -> Option<&[u8]> {
    let ptr = *payload.first()? as usize;
    let s = payload.get(1 + ptr..)?;
    let len = ((usize::from(*s.get(1)? & 0x0f)) << 8) | usize::from(*s.get(2)?);
    s.get(..3 + len)
}

/// What the PAT and PMTs say: which PIDs carry PMTs and which carry data.
#[derive(Clone, Debug, Default)]
struct Psi {
    pmt_pids: Vec<u16>,
    wanted: Vec<(u16, u8)>,
    have_pmt: bool,
}

impl Psi {
    /// Reads a PAT or PMT from `p`. Returns `true` if `p` is on a PSI PID.
    fn feed(&mut self, p: &TsPacket<'_>) -> bool {
        if p.pid == 0 {
            if p.start {
                if let Some(s) = section(p.payload) {
                    if s[0] == 0 && s.len() >= 12 {
                        for e in s[8..s.len() - 4].chunks_exact(4) {
                            let prog = u16::from_be_bytes([e[0], e[1]]);
                            let pid = u16::from_be_bytes([e[2] & 0x1f, e[3]]);
                            if prog != 0 && !self.pmt_pids.contains(&pid) {
                                self.pmt_pids.push(pid);
                            }
                        }
                    }
                }
            }
            return true;
        }
        if !self.pmt_pids.contains(&p.pid) {
            return false;
        }
        if !p.start {
            return true;
        }
        let Some(s) = section(p.payload) else {
            return true;
        };
        if s[0] != 2 || s.len() < 16 {
            return true;
        }
        self.have_pmt = true;
        let pil = (usize::from(s[10] & 0x0f) << 8) | usize::from(s[11]);
        let end = s.len() - 4;
        let mut i = 12 + pil;
        while i + 5 <= end {
            let st = s[i];
            let pid = u16::from_be_bytes([s[i + 1] & 0x1f, s[i + 2]]);
            let eil = (usize::from(s[i + 3] & 0x0f) << 8) | usize::from(s[i + 4]);
            if (st == 0x15 || st == 0x06) && !self.wanted.iter().any(|w| w.0 == pid) {
                self.wanted.push((pid, st));
            }
            i += 5 + eil;
        }
        true
    }

    fn stream_type(&self, pid: u16) -> Option<u8> {
        self.wanted.iter().find(|w| w.0 == pid).map(|w| w.1)
    }
}

/// A PES packet being reassembled.
#[derive(Clone, Debug)]
struct Pending {
    pid: u16,
    stream_type: u8,
    stream_id: u8,
    pts: Option<u64>,
    /// Payload bytes the PES header announces, when it gives a length.
    expected: Option<usize>,
    data: Vec<u8>,
}

/// Reads the 33-bit time stamp in a PES header's 5-byte PTS or DTS field.
fn read_timestamp(b: &[u8]) -> Option<u64> {
    let b = b.get(..5)?;
    Some(
        (u64::from(b[0] >> 1 & 0x07) << 30)
            | (u64::from(b[1]) << 22)
            | (u64::from(b[2] >> 1) << 15)
            | (u64::from(b[3]) << 7)
            | u64::from(b[4] >> 1),
    )
}

/// Removes ISO 13818-1 metadata Access Unit cell headers (5 bytes each:
/// service id, sequence number, flags, 16-bit cell length) from a
/// synchronous metadata payload. Returns `None` when `data` does not
/// look like AU cells (it already starts with a KLV key, or the first
/// cell's length does not fit), so the payload is kept as it is.
fn strip_au_cells(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 5 || data.starts_with(&UL_PREFIX) {
        return None;
    }
    let first = usize::from(u16::from_be_bytes([data[3], data[4]]));
    if first == 0 || 5 + first > data.len() || !data[5..].starts_with(&UL_PREFIX) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    let mut rest = data;
    while rest.len() >= 5 {
        let len = usize::from(u16::from_be_bytes([rest[3], rest[4]]));
        let end = (5 + len).min(rest.len());
        out.extend_from_slice(&rest[5..end]);
        rest = &rest[end..];
    }
    Some(out)
}

/// Turns the TS packets of data PIDs into complete PES payloads.
#[derive(Clone, Debug, Default)]
struct Assembler {
    open: Vec<Pending>,
    done: VecDeque<KlvUnit>,
}

impl Assembler {
    fn packet(&mut self, pid: u16, stream_type: u8, start: bool, payload: &[u8]) {
        if start {
            self.close(pid);
            let pl = payload;
            if pl.len() < 6 || pl[..3] != [0, 0, 1] {
                return;
            }
            let sid = pl[3];
            // Audio and video never carry KLV; skip them when there is no PMT.
            if (0xC0..=0xEF).contains(&sid) {
                return;
            }
            let pes_len = usize::from(u16::from_be_bytes([pl[4], pl[5]]));
            let no_ext = matches!(sid, 0xBC | 0xBE | 0xBF | 0xF0 | 0xF1 | 0xF2 | 0xF8 | 0xFF);
            let (pts, header, data) = if no_ext {
                (None, 0, &pl[6..])
            } else {
                let Some(&hlen) = pl.get(8) else { return };
                let hlen = usize::from(hlen);
                let Some(d) = pl.get(9 + hlen..) else { return };
                let pts = if pl[7] & 0x80 != 0 && hlen >= 5 {
                    read_timestamp(&pl[9..])
                } else {
                    None
                };
                (pts, 3 + hlen, d)
            };
            let expected = if pes_len == 0 {
                None
            } else {
                Some(pes_len.saturating_sub(header))
            };
            self.open.push(Pending {
                pid,
                stream_type,
                stream_id: sid,
                pts,
                expected,
                data: data.to_vec(),
            });
            self.check(pid);
        } else if let Some(p) = self.open.iter_mut().find(|p| p.pid == pid) {
            p.data.extend_from_slice(payload);
            self.check(pid);
        }
    }

    /// Closes the PES on `pid` once it has all the bytes its header announced.
    fn check(&mut self, pid: u16) {
        let full = self
            .open
            .iter()
            .any(|p| p.pid == pid && p.expected.is_some_and(|e| p.data.len() >= e));
        if full {
            self.close(pid);
        }
    }

    fn close(&mut self, pid: u16) {
        let Some(i) = self.open.iter().position(|p| p.pid == pid) else {
            return;
        };
        let mut p = self.open.swap_remove(i);
        if let Some(e) = p.expected {
            p.data.truncate(e);
        }
        if p.stream_id == 0xFC || p.stream_type == 0x15 {
            if let Some(d) = strip_au_cells(&p.data) {
                p.data = d;
            }
        }
        if p.data.is_empty() {
            return;
        }
        self.done.push_back(KlvUnit {
            pid: p.pid,
            stream_type: p.stream_type,
            stream_id: p.stream_id,
            pts: p.pts,
            data: p.data,
        });
    }

    fn close_all(&mut self) {
        let mut pids: Vec<u16> = self.open.iter().map(|p| p.pid).collect();
        pids.sort_unstable();
        for pid in pids {
            self.close(pid);
        }
    }
}

/// Demuxes a whole file. Returns the PSI and the units in file order.
fn demux_file(buf: &[u8]) -> (Psi, Vec<KlvUnit>) {
    let mut psi = Psi::default();
    if !is_transport_stream(buf) {
        return (psi, Vec::new());
    }
    // Pass 1: PAT, then the PMTs it names, so data that comes before the
    // first PMT is not lost.
    for p in packets(buf) {
        psi.feed(&p);
    }
    // Pass 2: reassemble PES payloads.
    let any = !psi.have_pmt;
    let mut asm = Assembler::default();
    for p in packets(buf) {
        if p.pid == 0 || p.pid == 0x1fff || psi.pmt_pids.contains(&p.pid) {
            continue;
        }
        let st = match psi.stream_type(p.pid) {
            Some(st) => st,
            None if any => 0,
            None => continue,
        };
        asm.packet(p.pid, st, p.start, p.payload);
    }
    asm.close_all();
    (psi, asm.done.into())
}

/// Every PES payload of the metadata and private data streams, in file
/// order, with its PID and PTS. Empty when the input is not a transport
/// stream. Without a PMT, every PID that is not audio or video is read.
pub fn klv_units(buf: &[u8]) -> Vec<KlvUnit> {
    demux_file(buf).1
}

/// The data streams in a transport stream, in PID order. Empty when the
/// input is not a transport stream or declares no metadata or private
/// data stream.
pub fn klv_streams(buf: &[u8]) -> Vec<DataStream> {
    let (psi, units) = demux_file(buf);
    let mut out: Vec<DataStream> = psi
        .wanted
        .iter()
        .map(|&(pid, stream_type)| DataStream {
            pid,
            stream_type,
            data: Vec::new(),
            pes_packets: 0,
        })
        .collect();
    for u in units {
        let idx = match out.iter().position(|s| s.pid == u.pid) {
            Some(i) => i,
            None => {
                out.push(DataStream {
                    pid: u.pid,
                    stream_type: u.stream_type,
                    data: Vec::new(),
                    pes_packets: 0,
                });
                out.len() - 1
            }
        };
        out[idx].data.extend_from_slice(&u.data);
        out[idx].pes_packets += 1;
    }
    out.sort_by_key(|s| s.pid);
    out
}

/// Takes KLV out of a transport stream that arrives in pieces, such as a
/// UDP multicast feed. Push bytes as they come and pop units as they
/// complete. Chunks may split packets anywhere; the demuxer finds the
/// 0x47 sync byte again after garbage or a lost datagram.
///
/// Data PIDs are learned from the PAT and PMT in the stream, so data that
/// arrives before the first PMT is skipped. A PES packet that gives its
/// length is returned as soon as it is complete; one that does not (PES
/// length 0) is returned when the next one on its PID starts, or on
/// [`Demuxer::flush`].
///
/// ```
/// use misb::ts::{Demuxer, Muxer};
///
/// let mut mux = Muxer::synchronous(0x1F1);
/// let mut feed = mux.tables();
/// feed.extend(mux.write(&misb::st0601::Builder::new(1).finish(), Some(3_000)));
///
/// let mut demux = Demuxer::new();
/// for datagram in feed.chunks(1316) {
///     demux.push(datagram);
///     while let Some(unit) = demux.pop() {
///         assert_eq!(unit.pid, 0x1F1);
///         assert_eq!(unit.pts, Some(3_000));
///     }
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Demuxer {
    buf: Vec<u8>,
    in_sync: bool,
    psi: Psi,
    asm: Assembler,
}

impl Demuxer {
    /// A demuxer that has seen nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds the next bytes of the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.scan(false);
    }

    fn scan(&mut self, end: bool) {
        let mut i = 0;
        while self.buf.len() - i >= PACKET_SIZE {
            // In sync, a packet starts here if the next one starts with the
            // sync byte too (or has not arrived yet). Out of sync, the next
            // two must, and the demuxer waits for them unless the stream
            // has ended.
            let look = if self.in_sync { 1 } else { 2 };
            let mut sync = self.buf[i] == 0x47;
            let mut wait = false;
            for k in 1..=look {
                match self.buf.get(i + k * PACKET_SIZE) {
                    Some(&b) => sync &= b == 0x47,
                    None => wait |= !self.in_sync && !end,
                }
            }
            if sync && wait {
                break;
            }
            self.in_sync = sync;
            if !sync {
                i += 1;
                continue;
            }
            if let Some(p) = parse_packet(&self.buf[i..i + PACKET_SIZE]) {
                if !self.psi.feed(&p) && p.pid != 0x1fff {
                    if let Some(st) = self.psi.stream_type(p.pid) {
                        self.asm.packet(p.pid, st, p.start, p.payload);
                    }
                }
            }
            i += PACKET_SIZE;
        }
        self.buf.drain(..i);
    }

    /// The next completed unit, if any.
    pub fn pop(&mut self) -> Option<KlvUnit> {
        self.asm.done.pop_front()
    }

    /// Closes every PES packet still open (end of stream). Their units
    /// are then available from [`Demuxer::pop`].
    pub fn flush(&mut self) {
        self.scan(true);
        self.buf.clear();
        self.asm.close_all();
    }

    /// The data PIDs and their `stream_type`, from the PMTs seen so far.
    pub fn data_pids(&self) -> impl Iterator<Item = (u16, u8)> + '_ {
        self.psi.wanted.iter().copied()
    }
}

/// CRC-32/MPEG-2, the checksum on PSI sections.
pub fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Writes KLV into a transport stream: one program whose only stream is
/// the KLV, on a PID you choose. Mux it next to video with your encoder's
/// PIDs, or use it alone for test files and simulators.
///
/// - [`Muxer::new`]: asynchronous KLV as MISB ST 1402 describes it,
///   `stream_type` 0x06, `stream_id` 0xBD, a `KLVA` registration
///   descriptor. The PTS is optional.
/// - [`Muxer::synchronous`]: synchronous KLV, `stream_type` 0x15,
///   `stream_id` 0xFC, a metadata descriptor with `KLVA` format, each
///   payload wrapped in one metadata AU cell. Give every packet the PTS
///   of the video frame it describes.
///
/// The PMT sets PCR PID 0x1FFF (no PCR), since the program has no clock
/// of its own; when you mux this next to video, write your own PMT.
#[derive(Clone, Debug)]
pub struct Muxer {
    pid: u16,
    pmt_pid: u16,
    sync: bool,
    cc_pat: u8,
    cc_pmt: u8,
    cc_data: u8,
    au_sequence: u8,
}

impl Muxer {
    /// Asynchronous KLV on `pid` (0x0010 to 0x1FFE).
    pub fn new(pid: u16) -> Self {
        Muxer {
            pid: pid & 0x1fff,
            pmt_pid: if pid & 0x1fff == 0x1000 {
                0x1001
            } else {
                0x1000
            },
            sync: false,
            cc_pat: 0,
            cc_pmt: 0,
            cc_data: 0,
            au_sequence: 0,
        }
    }

    /// Synchronous KLV on `pid` (0x0010 to 0x1FFE).
    pub fn synchronous(pid: u16) -> Self {
        Muxer {
            sync: true,
            ..Muxer::new(pid)
        }
    }

    /// Changes the PMT PID (default 0x1000).
    pub fn pmt_pid(mut self, pid: u16) -> Self {
        self.pmt_pid = pid & 0x1fff;
        self
    }

    /// The PAT and PMT, two 188-byte packets. Write them first, and again
    /// every so often in a live feed so receivers can join.
    pub fn tables(&mut self) -> Vec<u8> {
        let mut pat = vec![0x00, 0xB0, 0, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01];
        pat.extend_from_slice(&(0xE000 | self.pmt_pid).to_be_bytes());
        let mut out = psi_packet(0, &mut self.cc_pat, pat);

        let (stream_type, prog_info, es_info): (u8, &[u8], Vec<u8>) = if self.sync {
            // metadata_descriptor (ISO 13818-1 2.6.60): application format
            // 0xFFFF + "KLVA", metadata format 0xFF + "KLVA", service id 0,
            // decoder_config_flags 0, DSM-CC flag 0, reserved bits.
            let mut d = vec![0x26, 0, 0xFF, 0xFF];
            d.extend_from_slice(b"KLVA");
            d.push(0xFF);
            d.extend_from_slice(b"KLVA");
            d.extend_from_slice(&[0x00, 0x0F]);
            d[1] = (d.len() - 2) as u8;
            (0x15, &[], d)
        } else {
            let mut d = vec![0x05, 4];
            d.extend_from_slice(b"KLVA");
            (0x06, &[], d)
        };
        let mut pmt = vec![0x02, 0xB0, 0, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xFF, 0xFF];
        pmt.extend_from_slice(&(0xF000 | prog_info.len() as u16).to_be_bytes());
        pmt.extend_from_slice(prog_info);
        pmt.push(stream_type);
        pmt.extend_from_slice(&(0xE000 | self.pid).to_be_bytes());
        pmt.extend_from_slice(&(0xF000 | es_info.len() as u16).to_be_bytes());
        pmt.extend_from_slice(&es_info);
        out.extend(psi_packet(self.pmt_pid, &mut self.cc_pmt, pmt));
        out
    }

    /// One PES packet carrying `klv` (one or more whole KLV packets),
    /// split into 188-byte TS packets. `pts` is in 90 kHz ticks (only the
    /// low 33 bits are kept).
    pub fn write(&mut self, klv: &[u8], pts: Option<u64>) -> Vec<u8> {
        let mut payload = Vec::with_capacity(klv.len() + 5);
        if self.sync {
            // One AU cell per call (cells are at most 65,535 bytes; split
            // larger payloads into several cells).
            let mut chunks = klv.chunks(0xFFFF).peekable();
            let mut first = true;
            while let Some(c) = chunks.next() {
                let last = chunks.peek().is_none();
                let frag: u8 = match (first, last) {
                    (true, true) => 0b11,
                    (true, false) => 0b10,
                    (false, true) => 0b01,
                    (false, false) => 0b00,
                };
                // cell_fragment_indication, decoder_config_flag 0,
                // random_access_indicator 1, reserved 1111.
                payload.extend_from_slice(&[0x00, self.au_sequence, frag << 6 | 0x1F]);
                payload.extend_from_slice(&(c.len() as u16).to_be_bytes());
                payload.extend_from_slice(c);
                first = false;
            }
            self.au_sequence = self.au_sequence.wrapping_add(1);
        } else {
            payload.extend_from_slice(klv);
        }
        let sid = if self.sync { 0xFC } else { 0xBD };
        let mut pes = vec![0x00, 0x00, 0x01, sid, 0, 0, 0x84];
        match pts {
            Some(t) => {
                let t = t & 0x1_FFFF_FFFF;
                pes.extend_from_slice(&[
                    0x80,
                    5,
                    0x21 | ((t >> 29) as u8 & 0x0E),
                    (t >> 22) as u8,
                    ((t >> 14) as u8 & 0xFE) | 1,
                    (t >> 7) as u8,
                    ((t << 1) as u8 & 0xFE) | 1,
                ]);
            }
            None => pes.extend_from_slice(&[0x00, 0]),
        }
        let len = pes.len() - 6 + payload.len();
        if len <= 0xFFFF {
            pes[4..6].copy_from_slice(&(len as u16).to_be_bytes());
        }
        pes.extend_from_slice(&payload);
        packetize(self.pid, &mut self.cc_data, &pes)
    }
}

/// A PSI section (without CRC) as one TS packet: fills in the section
/// length, appends the CRC, pads with 0xFF.
fn psi_packet(pid: u16, cc: &mut u8, mut s: Vec<u8>) -> Vec<u8> {
    let len = s.len() - 3 + 4;
    s[1] = 0xB0 | (len >> 8) as u8 & 0x0F;
    s[2] = len as u8;
    let crc = crc32_mpeg2(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    let mut p = vec![
        0x47,
        0x40 | (pid >> 8) as u8 & 0x1f,
        pid as u8,
        0x10 | *cc,
        0,
    ];
    *cc = (*cc + 1) & 0x0F;
    p.extend_from_slice(&s);
    p.resize(PACKET_SIZE, 0xFF);
    p
}

/// Splits a PES packet into TS packets, padding the last one with an
/// adaptation field.
fn packetize(pid: u16, cc: &mut u8, pes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((pes.len() / 184 + 1) * PACKET_SIZE);
    let mut rest = pes;
    let mut first = true;
    while !rest.is_empty() {
        let pusi = if first { 0x40 } else { 0 };
        out.extend_from_slice(&[0x47, pusi | (pid >> 8) as u8 & 0x1f, pid as u8]);
        if rest.len() >= 184 {
            out.push(0x10 | *cc);
            out.extend_from_slice(&rest[..184]);
            rest = &rest[184..];
        } else {
            out.push(0x30 | *cc);
            let afl = 183 - rest.len();
            out.push(afl as u8);
            if afl > 0 {
                out.push(0x00);
                out.extend(core::iter::repeat(0xFF).take(afl - 1));
            }
            out.extend_from_slice(rest);
            rest = &[];
        }
        *cc = (*cc + 1) & 0x0F;
        first = false;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let u = klv_units(&file);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].pts, Some(0));
        assert_eq!(u[0].stream_id, 0xBD);
    }

    #[test]
    fn not_a_transport_stream() {
        assert!(!is_transport_stream(b"hello world"));
        assert!(klv_streams(&[0u8; 1000]).is_empty());
        assert!(klv_units(&[0u8; 1000]).is_empty());
    }

    #[test]
    fn crc_check_value() {
        assert_eq!(crc32_mpeg2(b"123456789"), 0x0376_E6E7);
    }

    #[test]
    fn tables_carry_valid_crcs() {
        for mut m in [Muxer::new(0x102), Muxer::synchronous(0x102)] {
            let t = m.tables();
            assert_eq!(t.len(), 2 * PACKET_SIZE);
            for p in t.chunks(PACKET_SIZE) {
                let s = section(&p[4..]).unwrap();
                // CRC over the whole section including its CRC is zero.
                assert_eq!(crc32_mpeg2(s), 0);
            }
        }
    }

    #[test]
    fn pts_round_trips_all_33_bits() {
        for pts in [0u64, 1, 90_000, 0x1_0000_0000, 0x1_FFFF_FFFF] {
            let mut m = Muxer::new(0x40);
            let mut f = m.tables();
            f.extend(m.write(b"\x06\x0e\x2b\x34", Some(pts)));
            let u = klv_units(&f);
            assert_eq!(u[0].pts, Some(pts));
        }
    }

    #[test]
    fn au_cells_are_removed() {
        let klv = crate::st0601::Builder::new(7).finish();
        let mut cells = vec![0, 3, 0xDF, 0, klv.len() as u8];
        cells.extend_from_slice(&klv);
        cells.extend_from_slice(&[0, 4, 0xDF, 0, klv.len() as u8]);
        cells.extend_from_slice(&klv);
        let out = strip_au_cells(&cells).unwrap();
        assert_eq!(out.len(), 2 * klv.len());
        assert_eq!(&out[..klv.len()], &klv[..]);
        // Plain KLV is left alone.
        assert!(strip_au_cells(&klv).is_none());
        // A first cell longer than the data is not a cell.
        assert!(strip_au_cells(&[0, 0, 0xDF, 0xFF, 0xFF, 6, 14, 43, 52]).is_none());
    }
}
