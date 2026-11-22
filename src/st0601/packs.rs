//! The ST 0601 packs and lists: the tags whose value is a small structure
//! of its own rather than one number or string.
//!
//! | Tag | Type | What it says |
//! |---|---|---|
//! | 81 | [`HorizonPixelPack`] | Where the horizon crosses the image, optionally on the ground |
//! | 115 | [`ControlCommand`] | A command sent to the aircraft (may repeat in one packet) |
//! | 116 | [`OidList`] | The ids of the control commands the aircraft acknowledged |
//! | 121 | [`OidList`] | The ids of the wavelengths the sensor is using now |
//! | 122 | [`CountryCodes`] | Overflight, operator and manufacturer country |
//! | 127 | [`FrameRate`] | Sensor frame rate as numerator and denominator (59.94 is 60000/1001) |
//! | 128 | [`Wavelengths`] | Wavelength bands beyond the predefined ones, in nanometres |
//! | 130 | [`AirbaseLocations`] | Take-off and recovery airfield |
//! | 138 | [`PayloadList`] | The sensors and payloads on board, by id, type and name |
//! | 139 | [`ActivePayloads`] | Which of those payloads are on |
//! | 141 | [`Waypoints`] | The flight plan: id, order, mode and position of each waypoint |
//! | 142 | [`ViewDomain`] | The range of azimuth, elevation and roll the sensor can reach |
//!
//! Each type has a `parse` that checks the whole value and returns an
//! [`Error`] for anything malformed, and the matching [`Packet`] accessor
//! (for example [`Packet::waypoints`]) returns `None` for a tag that is
//! missing or malformed. Lists are validated once in `parse` and then
//! iterate without allocating. With the `alloc` feature each type also
//! encodes, and [`Builder`] has a method per tag.
//!
//! Layouts follow ST 0601.19 and agree byte for byte with KWIVER's
//! ST 0601 test packet (`h264_klv.ts`) and with jmisb.
//!
//! [`Packet`]: super::Packet
//! [`Packet::waypoints`]: super::Packet::waypoints
//! [`Builder`]: super::Builder
//!
//! ```
//! # fn main() -> Result<(), misb::Error> {
//! use misb::st0601::{packs::{FrameRate, Waypoint}, Builder, Packet};
//! use misb::GeoPoint;
//!
//! let plan = [
//!     Waypoint::new(1, 1, GeoPoint { lat: 38.8895, lon: -77.0353, height: Some(450.0) }),
//!     Waypoint::new(2, 2, GeoPoint { lat: 38.8977, lon: -77.0365, height: None }),
//! ];
//! let mut b = Builder::new(1_700_000_000_000_000);
//! b.frame_rate(&FrameRate { numerator: 60_000, denominator: 1_001 })?
//!     .waypoints(&plan)?;
//! let bytes = b.finish();
//!
//! let (packet, _) = Packet::parse(&bytes)?;
//! assert!((packet.frame_rate().unwrap().fps().unwrap() - 59.94).abs() < 0.01);
//! let read: Vec<_> = packet.waypoints().unwrap().collect();
//! assert_eq!(read.len(), 2);
//! assert!((read[1].location.unwrap().lat - 38.8977).abs() < 1e-6);
//! # Ok(()) }
//! ```

use crate::imapb::Imapb;
use crate::klv::{be_uint, read_ber_length, read_ber_oid};
use crate::{Error, GeoPoint};

const LAT: Imapb = Imapb::new(-90.0, 90.0);
const LON: Imapb = Imapb::new(-180.0, 180.0);
const HAE: Imapb = Imapb::new(-900.0, 9000.0);
const WAVELENGTH: Imapb = Imapb::new(0.0, 1e9);
const ANGLE_360: Imapb = Imapb::new(0.0, 360.0);
const ANGLE_180: Imapb = Imapb::new(-180.0, 180.0);

/// Cursor over a pack's bytes.
#[derive(Clone)]
struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf }
    }
    fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    fn oid(&mut self) -> Result<u64, Error> {
        let (v, n) = read_ber_oid(self.buf)?;
        self.buf = &self.buf[n..];
        Ok(v)
    }
    fn len(&mut self) -> Result<usize, Error> {
        let (v, n) = read_ber_length(self.buf)?;
        self.buf = &self.buf[n..];
        Ok(v)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if n > self.buf.len() {
            return Err(Error::Truncated);
        }
        let (a, b) = self.buf.split_at(n);
        self.buf = b;
        Ok(a)
    }
    /// A BER length followed by that many bytes.
    fn lv(&mut self) -> Result<&'a [u8], Error> {
        let n = self.len()?;
        self.take(n)
    }
    fn text(&mut self, tag: u32) -> Result<&'a str, Error> {
        let raw = self.lv()?;
        core::str::from_utf8(raw).map_err(|_| Error::BadValue { tag })
    }
    fn end(&self) -> Result<(), Error> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(Error::BadLength)
        }
    }
}

/// Latitude, longitude (IMAPB, 4 bytes each) and an optional height above
/// the ellipsoid (IMAPB -900 to 9,000 m, 3 bytes): the location record of
/// tags 130 and 141.
fn read_location(raw: &[u8], tag: u32) -> Result<GeoPoint, Error> {
    if raw.len() != 8 && raw.len() != 11 {
        return Err(Error::BadLength);
    }
    let bad = Error::BadValue { tag };
    let lat = LAT.decode(&raw[0..4]).ok_or(bad)?;
    let lon = LON.decode(&raw[4..8]).ok_or(bad)?;
    let height = match raw.get(8..11) {
        Some(h) => Some(HAE.decode(h).ok_or(bad)?),
        None => None,
    };
    Ok(GeoPoint { lat, lon, height })
}

/// The 4-byte signed latitude and longitude of tags 13 and 14, which the
/// optional part of tag 81 uses. `0x80000000` means "not available".
fn read_flint_point(raw: &[u8]) -> Option<GeoPoint> {
    let lat = i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let lon = i32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]);
    if lat == i32::MIN || lon == i32::MIN {
        return None;
    }
    let max = i32::MAX as f64;
    Some(GeoPoint {
        lat: lat as f64 / max * 90.0,
        lon: lon as f64 / max * 180.0,
        height: None,
    })
}

/// Sensor Frame Rate Pack (tag 127): frames per second as a fraction, so
/// 29.97 and 59.94 are exact (30000/1001, 60000/1001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRate {
    /// Numerator.
    pub numerator: u64,
    /// Denominator; 1 when the sender leaves it out.
    pub denominator: u64,
}

impl FrameRate {
    /// Reads the pack: numerator and optional denominator, both BER-OID.
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        let numerator = r.oid()?;
        let denominator = if r.is_empty() { 1 } else { r.oid()? };
        r.end()?;
        Ok(FrameRate {
            numerator,
            denominator,
        })
    }

    /// Frames per second, `None` if the denominator is 0.
    pub fn fps(&self) -> Option<f64> {
        (self.denominator != 0).then(|| self.numerator as f64 / self.denominator as f64)
    }
}

/// Image Horizon Pixel Pack (tag 81): the two points where the horizon line
/// leaves the image, in percent of the image width (x) and height (y), and
/// optionally where those points are on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HorizonPixelPack {
    /// Start point, percent of image width from the left.
    pub x0: u8,
    /// Start point, percent of image height from the top.
    pub y0: u8,
    /// End point, percent of image width.
    pub x1: u8,
    /// End point, percent of image height.
    pub y1: u8,
    /// Ground position of the start point (20-byte form only).
    pub start: Option<GeoPoint>,
    /// Ground position of the end point (20-byte form only).
    pub end: Option<GeoPoint>,
}

impl HorizonPixelPack {
    /// Reads the 4-byte form (pixels only) or the 20-byte form (pixels and
    /// two positions). The positions use the signed 4-byte mapping of tags
    /// 13 and 14, as jmisb reads them; a position marked "not available"
    /// (`0x80000000`) comes back as `None`.
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        let (start, end) = match raw.len() {
            4 => (None, None),
            20 => (
                read_flint_point(&raw[4..12]),
                read_flint_point(&raw[12..20]),
            ),
            _ => return Err(Error::BadLength),
        };
        Ok(HorizonPixelPack {
            x0: raw[0],
            y0: raw[1],
            x1: raw[2],
            y1: raw[3],
            start,
            end,
        })
    }
}

/// Airbase Locations (tag 130): where the aircraft took off and where it
/// will land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirbaseLocations {
    /// Take-off location; `None` when the sender says it is unknown.
    pub takeoff: Option<GeoPoint>,
    /// Recovery location; `None` when unknown. When the sender leaves it
    /// out, ST 0601 says it is the take-off location, and that is what
    /// this holds.
    pub recovery: Option<GeoPoint>,
}

impl AirbaseLocations {
    /// Reads the pack: two length-prefixed location records, the second
    /// optional. A zero length means unknown.
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        let loc = |b: &[u8]| -> Result<Option<GeoPoint>, Error> {
            if b.is_empty() {
                Ok(None)
            } else {
                read_location(b, 130).map(Some)
            }
        };
        let takeoff = loc(r.lv()?)?;
        let recovery = if r.is_empty() { takeoff } else { loc(r.lv()?)? };
        r.end()?;
        Ok(AirbaseLocations { takeoff, recovery })
    }
}

/// Country Codes (tag 122).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryCodes<'a> {
    /// How the codes are written (the ST 0102 country coding method
    /// enumeration, for example 14 for GENC three-letter codes).
    pub coding_method: u64,
    /// Country being flown over.
    pub overflight: Option<&'a str>,
    /// Country of the operator.
    pub operator: Option<&'a str>,
    /// Country the aircraft was made in.
    pub manufacture: Option<&'a str>,
}

impl<'a> CountryCodes<'a> {
    /// Reads the pack: the coding method and up to three length-prefixed
    /// codes; trailing codes may be left out and a zero length means none.
    pub fn parse(raw: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        let method = r.lv()?;
        let coding_method = be_uint(method).ok_or(Error::BadValue { tag: 122 })?;
        let mut codes = [None; 3];
        for slot in codes.iter_mut() {
            if r.is_empty() {
                break;
            }
            let s = r.text(122)?;
            *slot = (!s.is_empty()).then_some(s);
        }
        r.end()?;
        Ok(CountryCodes {
            coding_method,
            overflight: codes[0],
            operator: codes[1],
            manufacture: codes[2],
        })
    }
}

/// One waypoint of the Waypoint List (tag 141).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    /// Waypoint id.
    pub id: u64,
    /// Prosecution order, signed, as the sender numbers it.
    pub order: i16,
    /// Info bits: bit 0 set = manual mode (clear = automated), bit 1 set =
    /// ad hoc source (clear = pre-planned). `None` when not sent.
    pub info: Option<u64>,
    /// Position; height is above the ellipsoid. `None` when not sent.
    pub location: Option<GeoPoint>,
}

impl Waypoint {
    /// A pre-planned, automated waypoint.
    pub fn new(id: u64, order: i16, location: GeoPoint) -> Self {
        Waypoint {
            id,
            order,
            info: Some(0),
            location: Some(location),
        }
    }

    /// Info bit 0: the aircraft flies to this waypoint under manual control.
    pub fn manual(&self) -> bool {
        self.info.unwrap_or(0) & 1 != 0
    }

    /// Info bit 1: the waypoint was added in flight, not in the plan.
    pub fn ad_hoc(&self) -> bool {
        self.info.unwrap_or(0) & 2 != 0
    }

    fn read(rec: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(rec);
        let id = r.oid()?;
        let o = r.take(2)?;
        let order = i16::from_be_bytes([o[0], o[1]]);
        let info = if r.is_empty() { None } else { Some(r.oid()?) };
        let location = if r.is_empty() {
            None
        } else {
            Some(read_location(r.buf, 141)?)
        };
        Ok(Waypoint {
            id,
            order,
            info,
            location,
        })
    }
}

/// One record of the Payload List (tag 138).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Payload<'a> {
    /// Payload id, the bit number in Active Payloads (tag 139).
    pub id: u64,
    /// Payload type (ST 0601 enumeration: 0 is electro-optical).
    pub payload_type: u64,
    /// Name, for example the sensor model.
    pub name: &'a str,
}

impl<'a> Payload<'a> {
    fn read(rec: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(rec);
        let id = r.oid()?;
        let payload_type = r.oid()?;
        let name = r.text(138)?;
        r.end()?;
        Ok(Payload {
            id,
            payload_type,
            name,
        })
    }
}

/// One record of the Wavelengths List (tag 128).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wavelength<'a> {
    /// Wavelength id, as used in Active Wavelength List (tag 121). Ids 1
    /// to 20 are predefined by ST 0601 and are not sent in this list.
    pub id: u64,
    /// Lower edge of the band, nanometres.
    pub min_nm: f64,
    /// Upper edge of the band, nanometres.
    pub max_nm: f64,
    /// Name of the band.
    pub name: &'a str,
}

impl<'a> Wavelength<'a> {
    fn read(rec: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(rec);
        let id = r.oid()?;
        let bad = Error::BadValue { tag: 128 };
        let min_nm = WAVELENGTH.decode(r.take(4)?).ok_or(bad)?;
        let max_nm = WAVELENGTH.decode(r.take(4)?).ok_or(bad)?;
        let name = core::str::from_utf8(r.buf).map_err(|_| bad)?;
        Ok(Wavelength {
            id,
            min_nm,
            max_nm,
            name,
        })
    }
}

macro_rules! series {
    ($(#[$m:meta])* $name:ident, $item:ident $(<$l:lifetime>)?) => {
        $(#[$m])*
        #[derive(Clone)]
        pub struct $name<'a> {
            r: Reader<'a>,
        }

        impl<'a> $name<'a> {
            /// Checks every record; the iterator then yields them all.
            pub fn parse(raw: &'a [u8]) -> Result<Self, Error> {
                let mut r = Reader::new(raw);
                while !r.is_empty() {
                    $item::read(r.lv()?)?;
                }
                Ok($name { r: Reader::new(raw) })
            }
        }

        impl<'a> Iterator for $name<'a> {
            type Item = $item $(<$l>)?;
            fn next(&mut self) -> Option<Self::Item> {
                if self.r.is_empty() {
                    return None;
                }
                let rec = self.r.lv().ok()?;
                $item::read(rec).ok()
            }
        }

        impl core::fmt::Debug for $name<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_list().entries(self.clone()).finish()
            }
        }
    };
}

series!(
    /// Waypoint List (tag 141): the waypoints in the order sent.
    Waypoints,
    Waypoint
);

series!(
    /// Wavelengths List (tag 128): bands beyond the predefined ones.
    Wavelengths,
    Wavelength<'a>
);

/// Payload List (tag 138): the payload count the sender gives and the
/// payload records.
#[derive(Clone)]
pub struct PayloadList<'a> {
    /// Number of payloads on the aircraft, as sent (it may exceed the
    /// number of records).
    pub count: u64,
    r: Reader<'a>,
}

impl<'a> PayloadList<'a> {
    /// Reads the count and checks every record.
    pub fn parse(raw: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        let count = r.oid()?;
        let start = r.clone();
        while !r.is_empty() {
            Payload::read(r.lv()?)?;
        }
        Ok(PayloadList { count, r: start })
    }

    /// The payload with id `id`.
    pub fn get(&self, id: u64) -> Option<Payload<'a>> {
        self.clone().find(|p| p.id == id)
    }
}

impl<'a> Iterator for PayloadList<'a> {
    type Item = Payload<'a>;
    fn next(&mut self) -> Option<Payload<'a>> {
        if self.r.is_empty() {
            return None;
        }
        Payload::read(self.r.lv().ok()?).ok()
    }
}

impl core::fmt::Debug for PayloadList<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PayloadList")
            .field("count", &self.count)
            .field("payloads", &DebugIter(self.clone()))
            .finish()
    }
}

struct DebugIter<I>(I);
impl<I: Iterator + Clone> core::fmt::Debug for DebugIter<I>
where
    I::Item: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.0.clone()).finish()
    }
}

/// A run of BER-OID integers: Active Wavelength List (tag 121) and
/// Control Command Verification List (tag 116).
#[derive(Clone)]
pub struct OidList<'a> {
    r: Reader<'a>,
}

impl<'a> OidList<'a> {
    /// Checks that `raw` is whole BER-OID integers.
    pub fn parse(raw: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        while !r.is_empty() {
            r.oid()?;
        }
        Ok(OidList {
            r: Reader::new(raw),
        })
    }
}

impl Iterator for OidList<'_> {
    type Item = u64;
    fn next(&mut self) -> Option<u64> {
        if self.r.is_empty() {
            return None;
        }
        self.r.oid().ok()
    }
}

impl core::fmt::Debug for OidList<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

/// Active Payloads (tag 139): a bit field, bit n set when payload n of the
/// Payload List is on. Any length; the last byte holds bits 0 to 7.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ActivePayloads<'a> {
    bits: &'a [u8],
}

impl<'a> ActivePayloads<'a> {
    /// Wraps the bit field.
    pub fn new(raw: &'a [u8]) -> Self {
        ActivePayloads { bits: raw }
    }

    /// Whether payload `id` is on.
    pub fn is_active(&self, id: u64) -> bool {
        let byte = (id / 8) as usize;
        if byte >= self.bits.len() {
            return false;
        }
        self.bits[self.bits.len() - 1 - byte] & (1 << (id % 8)) != 0
    }

    /// The ids of the active payloads, lowest first.
    pub fn ids(&self) -> impl Iterator<Item = u64> + 'a {
        let bits = self.bits;
        (0..bits.len() as u64 * 8).filter(move |&i| ActivePayloads { bits }.is_active(i))
    }
}

impl core::fmt::Debug for ActivePayloads<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.ids()).finish()
    }
}

/// Control Command (tag 115). A packet may carry several, one item each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlCommand<'a> {
    /// Command id, referred to by the Control Command Verification List.
    pub id: u64,
    /// The command, as text.
    pub text: &'a str,
    /// When it was sent, microseconds since 1970 UTC, if given.
    pub time: Option<u64>,
}

impl<'a> ControlCommand<'a> {
    /// Reads id (BER-OID), length-prefixed text and an optional 8-byte time.
    pub fn parse(raw: &'a [u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        let id = r.oid()?;
        let text = r.text(115)?;
        let time = if r.is_empty() {
            None
        } else {
            let t = r.take(8)?;
            Some(u64::from_be_bytes([
                t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7],
            ]))
        };
        r.end()?;
        Ok(ControlCommand { id, text, time })
    }
}

/// One axis of the View Domain: the sensor can point from `start` over
/// `range` degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewInterval {
    /// First angle of the domain, degrees.
    pub start: f64,
    /// Extent from `start`, degrees (0 to 360).
    pub range: f64,
}

/// View Domain (tag 142): how far the sensor can turn relative to the
/// platform. Azimuth and roll start in 0..360, elevation in -180..180.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewDomain {
    /// Relative azimuth domain.
    pub azimuth: Option<ViewInterval>,
    /// Relative elevation domain.
    pub elevation: Option<ViewInterval>,
    /// Relative roll domain.
    pub roll: Option<ViewInterval>,
}

impl ViewDomain {
    /// Reads up to three length-prefixed intervals, each a start and a
    /// range of equal IMAPB length. A zero length, or an interval left off
    /// the end, is `None`.
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(raw);
        let mut out = [None; 3];
        for (i, slot) in out.iter_mut().enumerate() {
            if r.is_empty() {
                break;
            }
            let b = r.lv()?;
            if b.is_empty() {
                continue;
            }
            if b.len() % 2 != 0 || b.len() > 16 {
                return Err(Error::BadLength);
            }
            let half = b.len() / 2;
            let start_map = if i == 1 { ANGLE_180 } else { ANGLE_360 };
            let bad = Error::BadValue { tag: 142 };
            *slot = Some(ViewInterval {
                start: start_map.decode(&b[..half]).ok_or(bad)?,
                range: ANGLE_360.decode(&b[half..]).ok_or(bad)?,
            });
        }
        r.end()?;
        Ok(ViewDomain {
            azimuth: out[0],
            elevation: out[1],
            roll: out[2],
        })
    }
}

#[cfg(feature = "alloc")]
pub(crate) mod enc {
    use super::*;
    use crate::klv::{enc::push_ber_length, write_ber_oid};
    use alloc::vec::Vec;

    pub(crate) fn oid(out: &mut Vec<u8>, v: u64) -> Result<(), Error> {
        let mut b = [0u8; 10];
        let n = write_ber_oid(v, &mut b)?;
        out.extend_from_slice(&b[..n]);
        Ok(())
    }

    pub(crate) fn lv(out: &mut Vec<u8>, v: &[u8]) -> Result<(), Error> {
        push_ber_length(out, v.len())?;
        out.extend_from_slice(v);
        Ok(())
    }

    fn imapb(out: &mut Vec<u8>, m: Imapb, x: f64, len: usize, tag: u32) -> Result<(), Error> {
        let mut b = [0u8; 8];
        if !m.encode(x, &mut b[..len]) {
            return Err(Error::OutOfRange { tag });
        }
        out.extend_from_slice(&b[..len]);
        Ok(())
    }

    pub(crate) fn location(out: &mut Vec<u8>, p: &GeoPoint, tag: u32) -> Result<(), Error> {
        imapb(out, LAT, p.lat, 4, tag)?;
        imapb(out, LON, p.lon, 4, tag)?;
        if let Some(h) = p.height {
            imapb(out, HAE, h, 3, tag)?;
        }
        Ok(())
    }

    fn flint(v: Option<f64>, scale: f64, tag: u32) -> Result<[u8; 4], Error> {
        match v {
            None => Ok(i32::MIN.to_be_bytes()),
            Some(x) if x >= -scale && x <= scale => {
                Ok((crate::math::round(x / scale * i32::MAX as f64) as i32).to_be_bytes())
            }
            Some(_) => Err(Error::OutOfRange { tag }),
        }
    }

    impl FrameRate {
        /// Encodes the pack; the denominator is left out when it is 1.
        pub fn encode(&self) -> Result<Vec<u8>, Error> {
            let mut out = Vec::new();
            oid(&mut out, self.numerator)?;
            if self.denominator != 1 {
                oid(&mut out, self.denominator)?;
            }
            Ok(out)
        }
    }

    impl HorizonPixelPack {
        /// Encodes the pack: 4 bytes, or 20 when either position is set
        /// (a missing one is written as "not available").
        pub fn encode(&self) -> Result<Vec<u8>, Error> {
            let mut out = alloc::vec![self.x0, self.y0, self.x1, self.y1];
            if self.x0.max(self.y0).max(self.x1).max(self.y1) > 100 {
                return Err(Error::OutOfRange { tag: 81 });
            }
            if self.start.is_some() || self.end.is_some() {
                for p in [self.start, self.end] {
                    out.extend_from_slice(&flint(p.map(|p| p.lat), 90.0, 81)?);
                    out.extend_from_slice(&flint(p.map(|p| p.lon), 180.0, 81)?);
                }
            }
            Ok(out)
        }
    }

    impl AirbaseLocations {
        /// Encodes the pack; the recovery location is left out when it is
        /// the take-off location.
        pub fn encode(&self) -> Result<Vec<u8>, Error> {
            let mut out = Vec::new();
            let rec = |p: Option<GeoPoint>, out: &mut Vec<u8>| -> Result<(), Error> {
                let mut b = Vec::new();
                if let Some(p) = p {
                    location(&mut b, &p, 130)?;
                }
                lv(out, &b)
            };
            rec(self.takeoff, &mut out)?;
            if self.recovery != self.takeoff || self.takeoff.is_none() {
                rec(self.recovery, &mut out)?;
            }
            Ok(out)
        }
    }

    impl CountryCodes<'_> {
        /// Encodes the pack, leaving off trailing codes that are `None`.
        pub fn encode(&self) -> Result<Vec<u8>, Error> {
            let mut out = Vec::new();
            lv(&mut out, &crate::klv::enc::min_uint(self.coding_method))?;
            let codes = [self.overflight, self.operator, self.manufacture];
            let n = codes.iter().rposition(Option::is_some).map_or(1, |i| i + 1);
            for c in &codes[..n] {
                lv(&mut out, c.unwrap_or("").as_bytes())?;
            }
            Ok(out)
        }
    }

    impl Waypoint {
        fn encode(&self) -> Result<Vec<u8>, Error> {
            let mut out = Vec::new();
            oid(&mut out, self.id)?;
            out.extend_from_slice(&self.order.to_be_bytes());
            match (self.info, self.location) {
                (_, Some(p)) => {
                    oid(&mut out, self.info.unwrap_or(0))?;
                    location(&mut out, &p, 141)?;
                }
                (Some(i), None) => oid(&mut out, i)?,
                (None, None) => {}
            }
            Ok(out)
        }
    }

    /// Encodes a Waypoint List (tag 141).
    pub fn waypoints(list: &[Waypoint]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for w in list {
            lv(&mut out, &w.encode()?)?;
        }
        Ok(out)
    }

    /// Encodes a Payload List (tag 138); the count is the number of records.
    pub fn payloads(list: &[Payload<'_>]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        oid(&mut out, list.len() as u64)?;
        for p in list {
            let mut rec = Vec::new();
            oid(&mut rec, p.id)?;
            oid(&mut rec, p.payload_type)?;
            lv(&mut rec, p.name.as_bytes())?;
            lv(&mut out, &rec)?;
        }
        Ok(out)
    }

    /// Encodes a Wavelengths List (tag 128).
    pub fn wavelengths(list: &[Wavelength<'_>]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for w in list {
            let mut rec = Vec::new();
            oid(&mut rec, w.id)?;
            imapb(&mut rec, WAVELENGTH, w.min_nm, 4, 128)?;
            imapb(&mut rec, WAVELENGTH, w.max_nm, 4, 128)?;
            rec.extend_from_slice(w.name.as_bytes());
            lv(&mut out, &rec)?;
        }
        Ok(out)
    }

    /// Encodes a run of BER-OID integers (tags 116 and 121).
    pub fn oid_list(ids: &[u64]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for &i in ids {
            oid(&mut out, i)?;
        }
        Ok(out)
    }

    /// Encodes Active Payloads (tag 139) from the ids that are on, in the
    /// fewest bytes.
    pub fn active_payloads(ids: &[u64]) -> Result<Vec<u8>, Error> {
        let top = ids.iter().copied().max().unwrap_or(0);
        if top >= 1 << 20 {
            return Err(Error::OutOfRange { tag: 139 });
        }
        let n = top as usize / 8 + 1;
        let mut out = alloc::vec![0u8; n];
        for &i in ids {
            out[n - 1 - i as usize / 8] |= 1 << (i % 8);
        }
        Ok(out)
    }

    impl ControlCommand<'_> {
        /// Encodes the command (text at most 127 bytes, as ST 0601 asks).
        pub fn encode(&self) -> Result<Vec<u8>, Error> {
            if self.text.len() > 127 {
                return Err(Error::BadValue { tag: 115 });
            }
            let mut out = Vec::new();
            oid(&mut out, self.id)?;
            lv(&mut out, self.text.as_bytes())?;
            if let Some(t) = self.time {
                out.extend_from_slice(&t.to_be_bytes());
            }
            Ok(out)
        }
    }

    impl ViewDomain {
        /// Encodes the pack with 3-byte IMAPB values, leaving off trailing
        /// axes that are `None`.
        pub fn encode(&self) -> Result<Vec<u8>, Error> {
            let axes = [self.azimuth, self.elevation, self.roll];
            let n = axes.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
            let mut out = Vec::new();
            for (i, a) in axes[..n].iter().enumerate() {
                let mut b = Vec::new();
                if let Some(a) = a {
                    let start_map = if i == 1 { ANGLE_180 } else { ANGLE_360 };
                    imapb(&mut b, start_map, a.start, 3, 142)?;
                    imapb(&mut b, ANGLE_360, a.range, 3, 142)?;
                }
                lv(&mut out, &b)?;
            }
            Ok(out)
        }
    }
}

#[cfg(feature = "alloc")]
pub use enc::waypoints as encode_waypoints;
#[cfg(feature = "alloc")]
pub use enc::{active_payloads as encode_active_payloads, oid_list as encode_oid_list};
#[cfg(feature = "alloc")]
pub use enc::{payloads as encode_payloads, wavelengths as encode_wavelengths};
