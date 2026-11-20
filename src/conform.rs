//! Conformance checks for a KLV stream: the verdict a ground station never
//! gives you.
//!
//! [`check_stream`] reads every ST 0601 packet in a byte stream and reports
//! what a decoder would otherwise accept in silence:
//!
//! - no ST 0601 packet at all (video without its metadata);
//! - packets that do not parse: bad key length, truncated value, checksum
//!   missing or wrong;
//! - the ST 0601 mandatory items: Precision Time Stamp (tag 2) present and
//!   first, UAS Datalink LS Version Number (tag 65) present, Checksum (tag 1)
//!   last;
//! - values the standard fixes the size of sent in another size (a 4-byte
//!   Target Width, a 7-byte time stamp), text that is not text;
//! - ST 1201 IMAPB values outside their mapping, and IMAPB tags that
//!   disagree with the older tag that carries the same quantity (tag 104
//!   against 75, 96 against 22, 90 against 6 ...), which is what a wrong
//!   mapping looks like on the wire;
//! - time stamps that go backwards, repeat, or were never set;
//! - positions no aircraft or terrain can have: below the Dead Sea, a frame
//!   center above Everest, a frame center beyond the sensor's horizon or
//!   at a different distance than the Slant Range says, a sensor that moves
//!   faster than 1,000 m/s between packets;
//! - nested VMTI sets whose target count does not match their targets.
//!
//! Every finding carries the packet number, its byte offset in the stream
//! and the tag, so a failure points at the bytes. Errors are violations of
//! ST 0601, ST 1201 or physics; warnings are legal but suspicious.
//!
//! ```
//! use misb::conform::check_stream;
//! use misb::st0601::{Builder, Tag};
//!
//! let mut b = Builder::new(1_700_000_000_000_000);
//! b.float(Tag::SensorLatitude, 38.9)?.float(Tag::SensorLongitude, -77.0)?;
//! let report = check_stream(&b.finish());
//! assert!(report.passed());
//!
//! // Video with no KLV at all fails.
//! assert!(!check_stream(&[0u8; 1000]).passed());
//! # Ok::<(), misb::Error>(())
//! ```

use std::fmt;
use std::format;
use std::string::String;
use std::vec::Vec;

use crate::klv::{self, Items};
use crate::st0601::{self, Kind, Packet, Tag};
use crate::{Error, GeoPoint};

/// How bad a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Legal, but almost always a fault in the encoder or the pipeline.
    Warning,
    /// Breaks ST 0601 or ST 1201, or describes something that cannot happen.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// One problem found in the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// Error or warning.
    pub severity: Severity,
    /// Stable identifier of the check, for example `CHECKSUM` or `TIME-BACKWARDS`.
    pub code: &'static str,
    /// Index of the ST 0601 packet (0-based), if the finding is about one.
    pub packet: Option<usize>,
    /// Byte offset of that packet's key in the input.
    pub offset: Option<usize>,
    /// The ST 0601 tag involved, if any.
    pub tag: Option<u32>,
    /// What is wrong, in one sentence, with the values involved.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.severity, self.code)?;
        if let Some(p) = self.packet {
            write!(f, " packet {p}")?;
        }
        if let Some(o) = self.offset {
            write!(f, " @{o:#x}")?;
        }
        if let Some(t) = self.tag {
            write!(f, " tag {t}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// The result of checking one stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// ST 0601 packets found (whether or not they parsed).
    pub packets: usize,
    /// Units of the older EG 0104 Predator universal set.
    pub eg0104_units: usize,
    /// KLV units with some other key (ST 0903 standalone, ST 0102, ...).
    pub other_units: usize,
    /// Everything found, in stream order.
    pub findings: Vec<Finding>,
}

impl Report {
    /// Number of errors.
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    /// Number of warnings.
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, s: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == s).count()
    }

    /// `true` when there is at least one ST 0601 packet and no error.
    pub fn passed(&self) -> bool {
        self.errors() == 0
    }
}

/// Checks every ST 0601 packet in a raw KLV byte stream (for example the
/// output of `ffmpeg -map 0:d -c copy -f data`). For an MPEG transport
/// stream, take the KLV out first with [`crate::ts::klv_streams`].
pub fn check_stream(stream: &[u8]) -> Report {
    let mut c = Checker::default();
    let base = stream.as_ptr() as usize;
    for unit in klv::split(stream) {
        let offset = unit.raw.as_ptr() as usize - base;
        c.unit(&unit, offset);
    }
    c.finish()
}

/// Checks packets one at a time, keeping what the time and motion checks
/// need from the previous packet.
#[derive(Clone, Debug, Default)]
pub struct Checker {
    report: Report,
    last_time: Option<u64>,
    last_pos: Option<(u64, GeoPoint)>,
}

/// Universal set key of MISB EG 0104 (Predator UAV Basic Universal Metadata Set).
const EG0104_KEY: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x01, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x02, 0x01, 0x01, 0x00, 0x00,
];
const MIN_TIME_US: u64 = 946_684_800_000_000; // 2000-01-01
const MAX_TIME_US: u64 = 4_102_444_800_000_000; // 2100-01-01
/// Lowest land surface on Earth is the Dead Sea shore, about -430 m MSL.
const LOWEST_MSL: f64 = -440.0;
/// Geoid undulation reaches about -106 m, so ellipsoid heights go lower.
const LOWEST_HAE: f64 = -550.0;
/// Everest, 8,849 m.
const HIGHEST_TERRAIN: f64 = 8_850.0;
/// Faster than any aircraft that carries a STANAG 4609 sensor (SR-71: ~980 m/s).
const MAX_SPEED: f64 = 1_000.0;

/// Pairs of tags that carry the same quantity at different precision: the
/// newer (often IMAPB) tag and the older one. When both are present they
/// must agree; when they do not, one of the mappings is wrong.
const SAME_QUANTITY: &[(Tag, Tag)] = &[
    (
        Tag::SensorEllipsoidHeightExtended,
        Tag::SensorEllipsoidHeight,
    ),
    (
        Tag::AlternatePlatformEllipsoidHeightExtended,
        Tag::AlternatePlatformEllipsoidHeight,
    ),
    (Tag::DensityAltitudeExtended, Tag::DensityAltitude),
    (Tag::TargetWidthExtended, Tag::TargetWidth),
    (Tag::PlatformPitchAngleFull, Tag::PlatformPitchAngle),
    (Tag::PlatformRollAngleFull, Tag::PlatformRollAngle),
    (Tag::PlatformAngleOfAttackFull, Tag::PlatformAngleOfAttack),
    (Tag::PlatformSideslipAngleFull, Tag::PlatformSideslipAngle),
];

impl Checker {
    /// A checker with no packets seen.
    pub fn new() -> Self {
        Self::default()
    }

    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        at: Option<(usize, usize)>,
        tag: Option<u32>,
        message: String,
    ) {
        self.report.findings.push(Finding {
            severity,
            code,
            packet: at.map(|a| a.0),
            offset: at.map(|a| a.1),
            tag,
            message,
        });
    }

    /// Checks one KLV unit found at byte `offset` of the stream. Units with
    /// a key other than ST 0601 are counted and otherwise ignored.
    pub fn unit(&mut self, unit: &klv::Unit<'_>, offset: usize) {
        if !klv::same_key(unit.key, &st0601::KEY) {
            if klv::same_key(unit.key, &EG0104_KEY) {
                self.report.eg0104_units += 1;
            } else {
                self.report.other_units += 1;
            }
            return;
        }
        let n = self.report.packets;
        self.report.packets += 1;
        let at = Some((n, offset));
        let packet = match Packet::parse(unit.raw) {
            Ok((p, _)) => p,
            Err(Error::Checksum { found, computed }) => {
                self.push(
                    Severity::Error,
                    "CHECKSUM",
                    at,
                    Some(1),
                    format!("checksum says {found:#06x}, the packet sums to {computed:#06x}; a decoder that checks drops this packet, one that does not trusts corrupt values"),
                );
                // Keep checking the content: the rest of the report is still useful.
                match Packet::parse_unverified(unit.raw) {
                    Ok((p, _)) => p,
                    Err(_) => return,
                }
            }
            Err(Error::MissingChecksum) => {
                self.push(
                    Severity::Error,
                    "CHECKSUM-NOT-LAST",
                    at,
                    Some(1),
                    "the packet does not end with a 2-byte Checksum item (tag 1), which ST 0601 makes mandatory and last".into(),
                );
                match Packet::parse_unverified(unit.raw) {
                    Ok((p, _)) => p,
                    Err(_) => return,
                }
            }
            Err(e) => {
                self.push(Severity::Error, "UNPARSEABLE", at, None, format!("{e}"));
                return;
            }
        };
        self.packet(&packet, n, offset);
    }

    fn packet(&mut self, p: &Packet<'_>, n: usize, offset: usize) {
        let at = Some((n, offset));
        // Item framing, order, duplicates, sizes.
        let mut seen = [false; 256];
        let mut first = true;
        for item in Items::new(p.body()) {
            let item = match item {
                Ok(i) => i,
                Err(e) => {
                    self.push(
                        Severity::Error,
                        "BAD-ITEM",
                        at,
                        None,
                        format!("an item inside the packet is malformed ({e}); everything after it is unreadable"),
                    );
                    break;
                }
            };
            if first && item.tag != 2 {
                self.push(
                    Severity::Error,
                    "TIMESTAMP-NOT-FIRST",
                    at,
                    Some(item.tag),
                    format!("the first item is tag {}, ST 0601 requires the Precision Time Stamp (tag 2) first", item.tag),
                );
            }
            first = false;
            if let Some(s) = seen.get_mut(item.tag as usize) {
                if *s && item.tag != 1 {
                    self.push(
                        Severity::Warning,
                        "DUPLICATE-TAG",
                        at,
                        Some(item.tag),
                        "the tag appears more than once; decoders keep either the first or the last".into(),
                    );
                }
                *s = true;
            }
            self.item_size(item.tag, item.value, at);
        }
        if !seen[2] {
            self.push(
                Severity::Error,
                "MISSING-TIMESTAMP",
                at,
                Some(2),
                "no Precision Time Stamp (tag 2): the metadata cannot be tied to a video frame"
                    .into(),
            );
        }
        if !seen[65] {
            self.push(
                Severity::Error,
                "MISSING-VERSION",
                at,
                Some(65),
                "no UAS Datalink LS Version Number (tag 65), which ST 0601 makes mandatory: a decoder cannot know which mappings apply".into(),
            );
        }

        self.same_quantity(p, at);
        self.time(p, at);
        self.geometry(p, at);
        self.vmti(p, at);
    }

    fn item_size(&mut self, tag: u32, raw: &[u8], at: Option<(usize, usize)>) {
        let Some(info) = st0601::tag_info(tag) else {
            if tag == 0 || tag > 143 {
                self.push(
                    Severity::Warning,
                    "UNKNOWN-TAG",
                    at,
                    Some(tag),
                    format!("tag {tag} is not defined in ST 0601.19"),
                );
            }
            return;
        };
        match info.kind {
            Kind::Time if tag == 2 && raw.len() != 8 => self.push(
                Severity::Error,
                "BAD-LENGTH",
                at,
                Some(tag),
                format!("{} is {} bytes, ST 0601 fixes it at 8", info.name, raw.len()),
            ),
            Kind::Time if raw.is_empty() || raw.len() > 8 => self.push(
                Severity::Error,
                "BAD-LENGTH",
                at,
                Some(tag),
                format!("{} is {} bytes", info.name, raw.len()),
            ),
            Kind::Linear { len, .. } if raw.len() != len as usize => self.push(
                Severity::Error,
                "BAD-LENGTH",
                at,
                Some(tag),
                format!(
                    "{} is {} bytes, ST 0601 maps it onto {len}; decoders that apply the standard scale read a different number",
                    info.name,
                    raw.len()
                ),
            ),
            Kind::Text if core::str::from_utf8(raw).is_err() => self.push(
                Severity::Error,
                "BAD-TEXT",
                at,
                Some(tag),
                format!("{} is not valid text", info.name),
            ),
            Kind::Imapb { min, max, .. } => self.imapb(tag, info.name, raw, min, max, at),
            Kind::Uint if tag == 65 && raw.len() != 1 => self.push(
                Severity::Error,
                "BAD-LENGTH",
                at,
                Some(tag),
                format!("{} is {} bytes, ST 0601 fixes it at 1", info.name, raw.len()),
            ),
            Kind::Uint | Kind::Int if raw.is_empty() || raw.len() > 8 => self.push(
                Severity::Error,
                "BAD-LENGTH",
                at,
                Some(tag),
                format!("{} is {} bytes", info.name, raw.len()),
            ),
            _ => {}
        }
    }

    fn imapb(
        &mut self,
        tag: u32,
        name: &str,
        raw: &[u8],
        min: f64,
        max: f64,
        at: Option<(usize, usize)>,
    ) {
        if raw.is_empty() || raw.len() > 8 {
            self.push(
                Severity::Error,
                "IMAPB-LENGTH",
                at,
                Some(tag),
                format!("{name} is {} bytes; ST 1201 IMAPB takes 1 to 8", raw.len()),
            );
            return;
        }
        let bits = 8 * raw.len() as u32;
        let y = klv::be_uint(raw).unwrap_or(0);
        if (y >> (bits - 2)) & 0b11 == 0b11 {
            self.push(
                Severity::Warning,
                "IMAPB-SPECIAL",
                at,
                Some(tag),
                format!("{name} carries an ST 1201 special value (top bits 11), not a number"),
            );
            return;
        }
        let m = crate::imapb::Imapb::new(min, max);
        if m.decode(raw).is_none() {
            self.push(
                Severity::Error,
                "IMAPB-RANGE",
                at,
                Some(tag),
                format!(
                    "{name}: integer {y:#x} maps above the ST 1201 range {min}..{max}; the encoder used a different mapping"
                ),
            );
        }
    }

    fn same_quantity(&mut self, p: &Packet<'_>, at: Option<(usize, usize)>) {
        for &(full, short) in SAME_QUANTITY {
            let (Some(a), Some(b)) = (p.float(full), p.float(short)) else {
                continue;
            };
            let step = |t: Tag, raw: Option<&[u8]>| match (t.info().kind, raw) {
                (Kind::Linear { len, min, max, .. }, _) => {
                    (max - min) / crate::math::pow2(8 * len as i32)
                }
                (Kind::Imapb { min, max, .. }, Some(r)) => crate::imapb::Imapb::new(min, max)
                    .resolution(r.len())
                    .unwrap_or(0.0),
                _ => 0.0,
            };
            let tol = 2.0 * (step(full, p.raw(full)) + step(short, p.raw(short))) + 1e-6;
            if (a - b).abs() > tol {
                self.push(
                    Severity::Error,
                    "MAPPING-MISMATCH",
                    at,
                    Some(full as u32),
                    format!(
                        "{} decodes to {a:.4} but {} (tag {}) says {b:.4}; both carry the same quantity, so one is wrong (a bad ST 1201 mapping or a value the encoder never updated)",
                        full.info().name,
                        short.info().name,
                        short as u32
                    ),
                );
            }
        }
    }

    fn time(&mut self, p: &Packet<'_>, at: Option<(usize, usize)>) {
        let Some(t) = p.timestamp() else { return };
        if !(MIN_TIME_US..MAX_TIME_US).contains(&t) {
            self.push(
                Severity::Error,
                "TIME-UNSET",
                at,
                Some(2),
                format!("Precision Time Stamp {t} us is {}; the clock was never set, so no frame can be matched to this metadata", civil(t)),
            );
        }
        if let Some(prev) = self.last_time {
            if t < prev {
                self.push(
                    Severity::Error,
                    "TIME-BACKWARDS",
                    at,
                    Some(2),
                    format!(
                        "time stamp {} is {:.3} s before the previous packet ({})",
                        civil(t),
                        (prev - t) as f64 / 1e6,
                        civil(prev)
                    ),
                );
            } else if t == prev {
                self.push(
                    Severity::Warning,
                    "TIME-REPEAT",
                    at,
                    Some(2),
                    "same time stamp as the previous packet".into(),
                );
            }
        }
        self.last_time = Some(t);
    }

    fn geometry(&mut self, p: &Packet<'_>, at: Option<(usize, usize)>) {
        let sensor = p.sensor_position();
        let fc = p.frame_center();
        if let Some(h) = p.float(Tag::SensorTrueAltitude) {
            if h < LOWEST_MSL {
                self.push(
                    Severity::Error,
                    "ALTITUDE-IMPOSSIBLE",
                    at,
                    Some(15),
                    format!("Sensor True Altitude {h:.1} m MSL is below the lowest land on Earth (Dead Sea, -430 m)"),
                );
            }
        }
        if let Some(h) = p.sensor_hae() {
            if h < LOWEST_HAE {
                self.push(
                    Severity::Error,
                    "ALTITUDE-IMPOSSIBLE",
                    at,
                    Some(75),
                    format!("sensor ellipsoid height {h:.1} m is below any point of the Earth's surface"),
                );
            }
        }
        if let Some(e) = p.float(Tag::FrameCenterElevation) {
            if !(LOWEST_MSL..=HIGHEST_TERRAIN).contains(&e) {
                self.push(
                    Severity::Error,
                    "TERRAIN-IMPOSSIBLE",
                    at,
                    Some(25),
                    format!("Frame Center Elevation {e:.1} m is terrain no place on Earth has (range -430 to 8,849 m)"),
                );
            }
        }
        for (pt, tag, what) in [(sensor, 13, "sensor"), (fc, 23, "frame center")] {
            if let Some(g) = pt {
                if g.lat == 0.0 && g.lon == 0.0 {
                    self.push(
                        Severity::Warning,
                        "NULL-ISLAND",
                        at,
                        Some(tag),
                        format!("{what} position is exactly 0, 0: a placeholder, not a fix"),
                    );
                }
            }
        }
        if let (Some(s), Some(c)) = (sensor, fc) {
            let ground = ground_distance(s, c);
            let above = s.height.unwrap_or(0.0) - c.height.unwrap_or(0.0);
            if above > 0.0 {
                let horizon = (2.0 * EARTH_R * above + above * above).sqrt();
                if ground > 1.1 * horizon + 1_000.0 {
                    self.push(
                        Severity::Error,
                        "BEYOND-HORIZON",
                        at,
                        Some(23),
                        format!(
                            "frame center is {:.1} km from the sensor, but from {above:.0} m above it the horizon is {:.1} km away: the sensor cannot see that point",
                            ground / 1e3,
                            horizon / 1e3
                        ),
                    );
                }
            }
            if let (Some(slant), Some(_), Some(_)) = (p.float(Tag::SlantRange), s.height, c.height)
            {
                let d = slant_distance(s, c);
                if slant > 0.0 && (d - slant).abs() > (0.2 * slant).max(250.0) {
                    self.push(
                        Severity::Error,
                        "SLANT-RANGE-MISMATCH",
                        at,
                        Some(21),
                        format!(
                            "Slant Range says {:.0} m but the sensor and frame center positions are {:.0} m apart",
                            slant, d
                        ),
                    );
                }
            }
        }
        if let (Some(s), Some(t)) = (sensor, p.timestamp()) {
            if let Some((t0, s0)) = self.last_pos {
                if t > t0 {
                    let dt = (t - t0) as f64 / 1e6;
                    let d = ground_distance(s0, s);
                    if dt >= 0.001 && d > 100.0 && d / dt > MAX_SPEED {
                        self.push(
                            Severity::Error,
                            "SPEED-IMPOSSIBLE",
                            at,
                            Some(13),
                            format!(
                                "the sensor moved {:.1} km in {dt:.3} s ({:.0} m/s) since the previous packet",
                                d / 1e3,
                                d / dt
                            ),
                        );
                    }
                }
            }
            self.last_pos = Some((t, s));
        }
    }

    fn vmti(&mut self, p: &Packet<'_>, at: Option<(usize, usize)>) {
        let Some(v) = p.vmti() else { return };
        let mut n = 0u64;
        for t in v.targets() {
            if t.is_err() {
                self.push(
                    Severity::Error,
                    "VMTI-BAD-TARGET",
                    at,
                    Some(74),
                    "a VTarget pack in the VMTI set is malformed".into(),
                );
                return;
            }
            n += 1;
        }
        if let Some(r) = v.targets_reported() {
            if r != n {
                self.push(
                    Severity::Error,
                    "VMTI-COUNT",
                    at,
                    Some(74),
                    format!("the VMTI set says {r} targets reported but carries {n}"),
                );
            }
        }
    }

    /// Ends the stream and returns the report. A stream with no ST 0601
    /// packet fails with `NO-KLV`.
    pub fn finish(mut self) -> Report {
        if self.report.packets == 0 && self.report.eg0104_units > 0 {
            let n = self.report.eg0104_units;
            self.push(
                Severity::Error,
                "ONLY-EG0104",
                None,
                None,
                format!("no ST 0601 packet, only {n} units of the 2006 EG 0104 Predator universal set: an ST 0601 decoder finds no metadata in this video"),
            );
        } else if self.report.packets == 0 {
            self.push(
                Severity::Error,
                "NO-KLV",
                None,
                None,
                "no ST 0601 packet in the input: without its KLV, drone video is just pictures"
                    .into(),
            );
        }
        self.report
    }
}

const EARTH_R: f64 = 6_371_008.8;

fn ground_distance(a: GeoPoint, b: GeoPoint) -> f64 {
    let (la1, la2) = (a.lat.to_radians(), b.lat.to_radians());
    let dla = la2 - la1;
    let dlo = (b.lon - a.lon).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * EARTH_R * h.sqrt().min(1.0).asin()
}

fn ecef(p: GeoPoint) -> (f64, f64, f64) {
    const A: f64 = 6_378_137.0;
    const E2: f64 = 6.694_379_990_14e-3;
    let (lat, lon) = (p.lat.to_radians(), p.lon.to_radians());
    let h = p.height.unwrap_or(0.0);
    let n = A / (1.0 - E2 * lat.sin().powi(2)).sqrt();
    (
        (n + h) * lat.cos() * lon.cos(),
        (n + h) * lat.cos() * lon.sin(),
        (n * (1.0 - E2) + h) * lat.sin(),
    )
}

fn slant_distance(a: GeoPoint, b: GeoPoint) -> f64 {
    let (x1, y1, z1) = ecef(a);
    let (x2, y2, z2) = ecef(b);
    ((x1 - x2).powi(2) + (y1 - y2).powi(2) + (z1 - z2).powi(2)).sqrt()
}

/// `YYYY-MM-DD hh:mm:ss UTC` for MISP microseconds.
fn civil(us: u64) -> String {
    let secs = us / 1_000_000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's days_from_civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::st0601::Builder;

    fn base(t: u64) -> Builder {
        let mut b = Builder::new(t);
        b.float(Tag::SensorLatitude, 38.9)
            .unwrap()
            .float(Tag::SensorLongitude, -77.0)
            .unwrap()
            .float(Tag::SensorTrueAltitude, 1500.0)
            .unwrap();
        b
    }

    fn codes(r: &Report) -> Vec<&'static str> {
        r.findings.iter().map(|f| f.code).collect()
    }

    #[test]
    fn clean_stream_passes() {
        let mut s = base(1_700_000_000_000_000).finish();
        s.extend(base(1_700_000_000_100_000).finish());
        let r = check_stream(&s);
        assert_eq!(r.packets, 2);
        assert!(r.passed(), "{:?}", r.findings);
        assert_eq!(r.warnings(), 0);
    }

    #[test]
    fn empty_input_is_no_klv() {
        let r = check_stream(b"just pictures");
        assert_eq!(codes(&r), ["NO-KLV"]);
        // An EG 0104 universal set is KLV, but not ST 0601.
        let mut legacy = EG0104_KEY.to_vec();
        legacy.extend_from_slice(&[4, 1, 2, 3, 4]);
        let r = check_stream(&legacy);
        assert_eq!(codes(&r), ["ONLY-EG0104"]);
    }

    #[test]
    fn checksum_and_order() {
        let mut p = base(1_700_000_000_000_000).finish();
        let n = p.len();
        p[n - 1] ^= 1;
        assert!(codes(&check_stream(&p)).contains(&"CHECKSUM"));

        // tag 13 first, no time stamp, no version
        let body = [13u8, 4, 0, 0, 0, 1];
        let mut pkt = st0601::KEY.to_vec();
        pkt.push((body.len() + 4) as u8);
        pkt.extend_from_slice(&body);
        pkt.extend_from_slice(&[1, 2]);
        let c = klv::checksum(&pkt);
        pkt.extend_from_slice(&c.to_be_bytes());
        let r = check_stream(&pkt);
        let c = codes(&r);
        assert!(c.contains(&"TIMESTAMP-NOT-FIRST"), "{c:?}");
        assert!(c.contains(&"MISSING-TIMESTAMP"));
        assert!(c.contains(&"MISSING-VERSION"));
    }

    #[test]
    fn wrong_length_and_mapping() {
        let mut b = base(1_700_000_000_000_000);
        b.raw(22, &[0, 0, 1, 0]).unwrap();
        assert!(codes(&check_stream(&b.finish())).contains(&"BAD-LENGTH"));

        let mut b = base(1_700_000_000_000_000);
        b.float(Tag::SensorEllipsoidHeight, 1500.0)
            .unwrap()
            .float(Tag::SensorEllipsoidHeightExtended, 1500.0)
            .unwrap();
        assert!(check_stream(&b.finish()).passed());

        // tag 104 written with tag 75's linear mapping
        let lin = ((1500.0 + 900.0) / 19900.0 * 65535.0) as u16;
        let mut b = base(1_700_000_000_000_000);
        b.float(Tag::SensorEllipsoidHeight, 1500.0)
            .unwrap()
            .raw(104, &lin.to_be_bytes())
            .unwrap();
        assert!(codes(&check_stream(&b.finish())).contains(&"MAPPING-MISMATCH"));
    }

    #[test]
    fn time_checks() {
        let mut s = base(1_700_000_001_000_000).finish();
        s.extend(base(1_700_000_000_000_000).finish());
        s.extend(base(1_700_000_000_000_000).finish());
        s.extend(base(5_000_000).finish());
        let c = codes(&check_stream(&s));
        assert!(c.contains(&"TIME-BACKWARDS"));
        assert!(c.contains(&"TIME-REPEAT"));
        assert!(c.contains(&"TIME-UNSET"));
        assert_eq!(civil(1_245_257_585_099_653), "2009-06-17 16:53:05 UTC");
    }

    #[test]
    fn physics() {
        // A frame center 400 km from a sensor at 1,500 m.
        let mut b = base(1_700_000_000_000_000);
        b.float(Tag::FrameCenterLatitude, 42.5)
            .unwrap()
            .float(Tag::FrameCenterLongitude, -77.0)
            .unwrap()
            .float(Tag::FrameCenterElevation, 100.0)
            .unwrap();
        let c = codes(&check_stream(&b.finish()));
        assert!(c.contains(&"BEYOND-HORIZON"), "{c:?}");

        // Slant range that does not match the geometry.
        let mut b = base(1_700_000_000_000_000);
        b.float(Tag::FrameCenterLatitude, 38.95)
            .unwrap()
            .float(Tag::FrameCenterLongitude, -77.0)
            .unwrap()
            .float(Tag::FrameCenterElevation, 100.0)
            .unwrap()
            .float(Tag::SlantRange, 20_000.0)
            .unwrap();
        assert!(codes(&check_stream(&b.finish())).contains(&"SLANT-RANGE-MISMATCH"));

        // Altitude 0 bytes = -900 m, terrain above Everest, teleport.
        let mut s = Builder::new(1_700_000_000_000_000);
        s.float(Tag::SensorLatitude, 10.0)
            .unwrap()
            .float(Tag::SensorLongitude, 10.0)
            .unwrap()
            .raw(15, &[0, 0])
            .unwrap()
            .float(Tag::FrameCenterElevation, 9_500.0)
            .unwrap();
        let mut stream = s.finish();
        let mut s2 = Builder::new(1_700_000_001_000_000);
        s2.float(Tag::SensorLatitude, 11.0)
            .unwrap()
            .float(Tag::SensorLongitude, 10.0)
            .unwrap();
        stream.extend(s2.finish());
        let c = codes(&check_stream(&stream));
        assert!(c.contains(&"ALTITUDE-IMPOSSIBLE"), "{c:?}");
        assert!(c.contains(&"TERRAIN-IMPOSSIBLE"));
        assert!(c.contains(&"SPEED-IMPOSSIBLE"));
    }
}
