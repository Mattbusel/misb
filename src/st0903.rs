//! MISB ST 0903 Video Moving Target Indicator (VMTI) Local Set: what a
//! video tracker detected in a frame (pixel boxes, ground positions,
//! confidence). It is nested in ST 0601 as tag 74 or sent as its own packet
//! with key [`KEY`].
//!
//! ```
//! # fn main() -> Result<(), misb::Error> {
//! use misb::st0903::{Builder, TargetBuilder, VmtiSet};
//!
//! let mut vmti = Builder::new(1_700_000_000_000_000);
//! vmti.system_name("tracker-1")?.frame_size(1280, 720)?;
//! let mut t = TargetBuilder::new(7);
//! t.centroid_pixel(1280 * 359 + 640)?.confidence(87)?.location(38.8895, -77.0353, 12.0)?;
//! vmti.target(&t)?;
//! let body = vmti.body();
//!
//! let set = VmtiSet::new(&body);
//! let target = set.targets().next().unwrap()?;
//! assert_eq!(target.id, 7);
//! assert_eq!(target.centroid_row_col(set.frame_width().unwrap()), Some((360, 640)));
//! let loc = target.location().unwrap();
//! assert!((loc.lat - 38.8895).abs() < 1e-6);
//! # Ok(()) }
//! ```

use crate::imapb::Imapb;
use crate::klv::{self, be_uint, read_ber_length, read_ber_oid, Items};
use crate::{Error, GeoPoint};

/// Universal key of a standalone VMTI Local Set packet.
pub const KEY: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x03, 0x06, 0x00, 0x00, 0x00,
];

/// The ST 0903 revision written by [`Builder`].
pub const VERSION: u64 = 6;

/// VMTI Local Set tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum Tag {
    /// Checksum (standalone packets only).
    Checksum = 1,
    /// Precision Time Stamp, microseconds since 1970 UTC.
    PrecisionTimeStamp = 2,
    /// VMTI System Name.
    SystemName = 3,
    /// VMTI LS Version Number.
    Version = 4,
    /// Total Number of Targets Detected in the frame.
    TotalTargetsDetected = 5,
    /// Number of Targets Reported in this set.
    TargetsReported = 6,
    /// Motion Imagery Frame Number.
    FrameNumber = 7,
    /// Frame Width, pixels.
    FrameWidth = 8,
    /// Frame Height, pixels.
    FrameHeight = 9,
    /// VMTI Source Sensor.
    SourceSensor = 10,
    /// VMTI Sensor Horizontal Field of View, degrees.
    HorizontalFov = 11,
    /// VMTI Sensor Vertical Field of View, degrees.
    VerticalFov = 12,
    /// MIIS ID (ST 1204).
    MiisId = 13,
    /// VTarget Series.
    VTargetSeries = 101,
    /// Algorithm Series.
    AlgorithmSeries = 102,
    /// Ontology Series.
    OntologySeries = 103,
}

const FOV: Imapb = Imapb::new(0.0, 180.0);
const OFFSET: Imapb = Imapb::new(-19.2, 19.2);
const HEIGHT: Imapb = Imapb::new(-900.0, 19000.0);
const LAT: Imapb = Imapb::new(-90.0, 90.0);
const LON: Imapb = Imapb::new(-180.0, 180.0);

/// A VMTI Local Set body, borrowed from the packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmtiSet<'a> {
    body: &'a [u8],
}

impl<'a> VmtiSet<'a> {
    /// Wraps a local set body (the value of ST 0601 tag 74).
    pub fn new(body: &'a [u8]) -> Self {
        VmtiSet { body }
    }

    /// Parses a standalone VMTI packet (key, length, items, checksum).
    /// Returns the set and the bytes used.
    pub fn parse(buf: &'a [u8]) -> Result<(VmtiSet<'a>, usize), Error> {
        let (body, used) = klv::parse_checked(buf, &KEY, true)?;
        Ok((VmtiSet { body }, used))
    }

    /// The raw body.
    pub fn body(&self) -> &'a [u8] {
        self.body
    }

    /// All items (tag and raw bytes).
    pub fn items(&self) -> Items<'a> {
        Items::new(self.body)
    }

    /// Raw bytes of `tag`.
    pub fn raw(&self, tag: Tag) -> Option<&'a [u8]> {
        klv::find(self.body, tag as u32)
    }

    fn uint(&self, tag: Tag) -> Option<u64> {
        self.raw(tag).and_then(be_uint)
    }

    /// Precision Time Stamp (tag 2), microseconds since 1970 UTC.
    pub fn timestamp(&self) -> Option<u64> {
        self.raw(Tag::PrecisionTimeStamp)
            .filter(|r| r.len() == 8)
            .and_then(be_uint)
    }

    /// VMTI System Name (tag 3).
    pub fn system_name(&self) -> Option<&'a str> {
        core::str::from_utf8(self.raw(Tag::SystemName)?).ok()
    }

    /// VMTI Source Sensor (tag 10).
    pub fn source_sensor(&self) -> Option<&'a str> {
        core::str::from_utf8(self.raw(Tag::SourceSensor)?).ok()
    }

    /// VMTI LS Version (tag 4).
    pub fn version(&self) -> Option<u64> {
        self.uint(Tag::Version)
    }

    /// Total Number of Targets Detected (tag 5).
    pub fn total_targets_detected(&self) -> Option<u64> {
        self.uint(Tag::TotalTargetsDetected)
    }

    /// Number of Targets Reported (tag 6).
    pub fn targets_reported(&self) -> Option<u64> {
        self.uint(Tag::TargetsReported)
    }

    /// Motion Imagery Frame Number (tag 7).
    pub fn frame_number(&self) -> Option<u64> {
        self.uint(Tag::FrameNumber)
    }

    /// Frame Width in pixels (tag 8).
    pub fn frame_width(&self) -> Option<u64> {
        self.uint(Tag::FrameWidth)
    }

    /// Frame Height in pixels (tag 9).
    pub fn frame_height(&self) -> Option<u64> {
        self.uint(Tag::FrameHeight)
    }

    /// Horizontal field of view in degrees (tag 11).
    pub fn horizontal_fov(&self) -> Option<f64> {
        FOV.decode(self.raw(Tag::HorizontalFov)?)
    }

    /// Vertical field of view in degrees (tag 12).
    pub fn vertical_fov(&self) -> Option<f64> {
        FOV.decode(self.raw(Tag::VerticalFov)?)
    }

    /// The targets in the VTarget Series (tag 101).
    pub fn targets(&self) -> Targets<'a> {
        Targets {
            rest: self.raw(Tag::VTargetSeries).unwrap_or(&[]),
            failed: false,
        }
    }
}

/// Iterator over the VTarget packs of a [`VmtiSet`].
#[derive(Clone, Debug)]
pub struct Targets<'a> {
    rest: &'a [u8],
    failed: bool,
}

impl<'a> Targets<'a> {
    fn step(&mut self) -> Result<VTarget<'a>, Error> {
        let (len, n) = read_ber_length(self.rest)?;
        let end = n.checked_add(len).ok_or(Error::BadLength)?;
        let pack = self.rest.get(n..end).ok_or(Error::Truncated)?;
        self.rest = &self.rest[end..];
        let (id, m) = read_ber_oid(pack)?;
        Ok(VTarget {
            id,
            body: &pack[m..],
        })
    }
}

impl<'a> Iterator for Targets<'a> {
    type Item = Result<VTarget<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() || self.failed {
            return None;
        }
        let r = self.step();
        self.failed = r.is_err();
        Some(r)
    }
}

/// VTarget pack tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum TargetTag {
    /// Target Centroid, as a pixel number.
    Centroid = 1,
    /// Boundary Top Left, as a pixel number.
    BoundaryTopLeft = 2,
    /// Boundary Bottom Right, as a pixel number.
    BoundaryBottomRight = 3,
    /// Target Priority (1 is highest).
    Priority = 4,
    /// Target Confidence Level, percent.
    Confidence = 5,
    /// Target History (frames tracked).
    History = 6,
    /// Percentage of Target Pixels.
    PercentagePixels = 7,
    /// Target Color, 0xRRGGBB.
    Color = 8,
    /// Target Intensity.
    Intensity = 9,
    /// Target Location Offset Latitude, degrees from the frame center.
    LocationOffsetLat = 10,
    /// Target Location Offset Longitude, degrees from the frame center.
    LocationOffsetLon = 11,
    /// Target Height, metres above the ellipsoid.
    Height = 12,
    /// Bounding Box Top Left Latitude Offset.
    BoxTopLeftLatOffset = 13,
    /// Bounding Box Top Left Longitude Offset.
    BoxTopLeftLonOffset = 14,
    /// Bounding Box Bottom Right Latitude Offset.
    BoxBottomRightLatOffset = 15,
    /// Bounding Box Bottom Right Longitude Offset.
    BoxBottomRightLonOffset = 16,
    /// Target Location pack (latitude, longitude, height).
    Location = 17,
    /// Target Boundary Series.
    BoundarySeries = 18,
    /// Centroid Pixel Row.
    CentroidRow = 19,
    /// Centroid Pixel Column.
    CentroidColumn = 20,
    /// FPA Index.
    FpaIndex = 21,
    /// Algorithm ID.
    AlgorithmId = 22,
}

/// One detection in a VMTI set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VTarget<'a> {
    /// Target ID, unique within the stream while the target is tracked.
    pub id: u64,
    body: &'a [u8],
}

impl<'a> VTarget<'a> {
    /// All items of the pack (tag and raw bytes).
    pub fn items(&self) -> Items<'a> {
        Items::new(self.body)
    }

    /// Raw bytes of `tag`.
    pub fn raw(&self, tag: TargetTag) -> Option<&'a [u8]> {
        klv::find(self.body, tag as u32)
    }

    fn uint(&self, tag: TargetTag) -> Option<u64> {
        self.raw(tag).and_then(be_uint)
    }

    /// Target Centroid pixel number (tag 1): `column + (row - 1) * width`, 1-based.
    pub fn centroid_pixel(&self) -> Option<u64> {
        self.uint(TargetTag::Centroid)
    }

    /// Centroid as 1-based `(row, column)`: tags 19 and 20 when present,
    /// otherwise derived from the pixel number and the frame width.
    pub fn centroid_row_col(&self, frame_width: u64) -> Option<(u64, u64)> {
        if let (Some(r), Some(c)) = (
            self.uint(TargetTag::CentroidRow),
            self.uint(TargetTag::CentroidColumn),
        ) {
            return Some((r, c));
        }
        pixel_to_row_col(self.centroid_pixel()?, frame_width)
    }

    /// Bounding box as pixel numbers (top left, bottom right).
    pub fn bounding_box(&self) -> Option<(u64, u64)> {
        Some((
            self.uint(TargetTag::BoundaryTopLeft)?,
            self.uint(TargetTag::BoundaryBottomRight)?,
        ))
    }

    /// Target Priority (tag 4).
    pub fn priority(&self) -> Option<u8> {
        self.uint(TargetTag::Priority).map(|v| v as u8)
    }

    /// Target Confidence Level in percent (tag 5).
    pub fn confidence(&self) -> Option<u8> {
        self.uint(TargetTag::Confidence).map(|v| v as u8)
    }

    /// Target History (tag 6).
    pub fn history(&self) -> Option<u64> {
        self.uint(TargetTag::History)
    }

    /// Target Color as 0xRRGGBB (tag 8).
    pub fn color(&self) -> Option<u32> {
        self.uint(TargetTag::Color).map(|v| v as u32)
    }

    /// Target Location Offset (tags 10, 11): degrees of latitude and
    /// longitude from the frame center.
    pub fn location_offset(&self) -> Option<(f64, f64)> {
        Some((
            OFFSET.decode(self.raw(TargetTag::LocationOffsetLat)?)?,
            OFFSET.decode(self.raw(TargetTag::LocationOffsetLon)?)?,
        ))
    }

    /// Target Height in metres above the ellipsoid (tag 12).
    pub fn height(&self) -> Option<f64> {
        HEIGHT.decode(self.raw(TargetTag::Height)?)
    }

    /// Target Location pack (tag 17): latitude, longitude and height.
    pub fn location(&self) -> Option<GeoPoint> {
        let p = self.raw(TargetTag::Location)?;
        if p.len() < 10 {
            return None;
        }
        Some(GeoPoint {
            lat: LAT.decode(&p[0..4])?,
            lon: LON.decode(&p[4..8])?,
            height: HEIGHT.decode(&p[8..10]),
        })
    }

    /// The ground position: tag 17 if present, else the offsets (tags 10,
    /// 11) added to `frame_center` (ST 0601 tags 23, 24).
    pub fn ground_position(&self, frame_center: Option<GeoPoint>) -> Option<GeoPoint> {
        if let Some(p) = self.location() {
            return Some(p);
        }
        let fc = frame_center?;
        let (dlat, dlon) = self.location_offset()?;
        Some(GeoPoint {
            lat: fc.lat + dlat,
            lon: fc.lon + dlon,
            height: self.height(),
        })
    }

    /// Algorithm ID (tag 22).
    pub fn algorithm_id(&self) -> Option<u64> {
        self.uint(TargetTag::AlgorithmId)
    }
}

/// Converts a 1-based ST 0903 pixel number into 1-based `(row, column)`.
pub fn pixel_to_row_col(pixel: u64, frame_width: u64) -> Option<(u64, u64)> {
    if pixel == 0 || frame_width == 0 {
        return None;
    }
    Some(((pixel - 1) / frame_width + 1, (pixel - 1) % frame_width + 1))
}

#[cfg(feature = "alloc")]
pub use builder::{Builder, TargetBuilder};

#[cfg(feature = "alloc")]
mod builder {
    use super::*;
    use crate::klv::enc;
    use alloc::vec::Vec;

    /// Encodes one VTarget pack.
    #[derive(Clone, Debug)]
    pub struct TargetBuilder {
        id: u64,
        body: Vec<u8>,
    }

    impl TargetBuilder {
        /// Starts a pack for target `id`.
        pub fn new(id: u64) -> Self {
            TargetBuilder {
                id,
                body: Vec::new(),
            }
        }

        /// Adds a raw item.
        pub fn raw(&mut self, tag: u32, value: &[u8]) -> Result<&mut Self, Error> {
            enc::push_item(&mut self.body, tag, value)?;
            Ok(self)
        }

        /// Target Centroid pixel number.
        pub fn centroid_pixel(&mut self, pixel: u64) -> Result<&mut Self, Error> {
            self.raw(1, &enc::min_uint(pixel))
        }

        /// Bounding box pixel numbers.
        pub fn bounding_box(
            &mut self,
            top_left: u64,
            bottom_right: u64,
        ) -> Result<&mut Self, Error> {
            self.raw(2, &enc::min_uint(top_left))?
                .raw(3, &enc::min_uint(bottom_right))
        }

        /// Target Priority, 1 to 255.
        pub fn priority(&mut self, p: u8) -> Result<&mut Self, Error> {
            self.raw(4, &[p])
        }

        /// Target Confidence Level, 0 to 100.
        pub fn confidence(&mut self, c: u8) -> Result<&mut Self, Error> {
            if c > 100 {
                return Err(Error::OutOfRange { tag: 5 });
            }
            self.raw(5, &[c])
        }

        /// Target History (frames tracked).
        pub fn history(&mut self, frames: u64) -> Result<&mut Self, Error> {
            self.raw(6, &enc::min_uint(frames))
        }

        /// Target Location pack: latitude, longitude, height above ellipsoid.
        pub fn location(&mut self, lat: f64, lon: f64, hae: f64) -> Result<&mut Self, Error> {
            let mut p = [0u8; 10];
            if !(LAT.encode(lat, &mut p[0..4])
                && LON.encode(lon, &mut p[4..8])
                && HEIGHT.encode(hae, &mut p[8..10]))
            {
                return Err(Error::OutOfRange { tag: 17 });
            }
            self.raw(17, &p)
        }

        /// Location offset from the frame center, degrees (each within +-19.2).
        pub fn location_offset(&mut self, dlat: f64, dlon: f64) -> Result<&mut Self, Error> {
            let mut a = [0u8; 3];
            let mut b = [0u8; 3];
            if !(OFFSET.encode(dlat, &mut a) && OFFSET.encode(dlon, &mut b)) {
                return Err(Error::OutOfRange { tag: 10 });
            }
            self.raw(10, &a)?.raw(11, &b)
        }

        /// Centroid row and column (1-based).
        pub fn centroid_row_col(&mut self, row: u64, col: u64) -> Result<&mut Self, Error> {
            self.raw(19, &enc::min_uint(row))?
                .raw(20, &enc::min_uint(col))
        }

        /// Algorithm ID.
        pub fn algorithm_id(&mut self, id: u64) -> Result<&mut Self, Error> {
            self.raw(22, &enc::min_uint(id))
        }

        fn pack(&self) -> Result<Vec<u8>, Error> {
            let mut b = [0u8; 10];
            let n = klv::write_ber_oid(self.id, &mut b)?;
            let mut v = b[..n].to_vec();
            v.extend_from_slice(&self.body);
            Ok(v)
        }
    }

    /// Encodes a VMTI Local Set, nested ([`Builder::body`]) or standalone
    /// ([`Builder::finish`]).
    #[derive(Clone, Debug)]
    pub struct Builder {
        header: Vec<u8>,
        series: Vec<u8>,
        targets: u64,
    }

    impl Builder {
        /// Starts a set with Precision Time Stamp `timestamp_us`.
        pub fn new(timestamp_us: u64) -> Self {
            let mut header = Vec::new();
            enc::push_item(&mut header, 2, &timestamp_us.to_be_bytes()).expect("small item");
            Builder {
                header,
                series: Vec::new(),
                targets: 0,
            }
        }

        /// Adds a raw header item.
        pub fn raw(&mut self, tag: u32, value: &[u8]) -> Result<&mut Self, Error> {
            enc::push_item(&mut self.header, tag, value)?;
            Ok(self)
        }

        /// VMTI System Name (at most 32 bytes).
        pub fn system_name(&mut self, name: &str) -> Result<&mut Self, Error> {
            if name.len() > 32 {
                return Err(Error::BadValue { tag: 3 });
            }
            self.raw(3, name.as_bytes())
        }

        /// VMTI Source Sensor (at most 32 bytes).
        pub fn source_sensor(&mut self, name: &str) -> Result<&mut Self, Error> {
            if name.len() > 32 {
                return Err(Error::BadValue { tag: 10 });
            }
            self.raw(10, name.as_bytes())
        }

        /// Frame width and height in pixels.
        pub fn frame_size(&mut self, width: u64, height: u64) -> Result<&mut Self, Error> {
            self.raw(8, &enc::min_uint(width))?
                .raw(9, &enc::min_uint(height))
        }

        /// Motion Imagery Frame Number.
        pub fn frame_number(&mut self, n: u64) -> Result<&mut Self, Error> {
            self.raw(7, &enc::min_uint(n))
        }

        /// Horizontal and vertical field of view in degrees.
        pub fn fov(&mut self, horizontal: f64, vertical: f64) -> Result<&mut Self, Error> {
            let mut h = [0u8; 2];
            let mut v = [0u8; 2];
            if !(FOV.encode(horizontal, &mut h) && FOV.encode(vertical, &mut v)) {
                return Err(Error::OutOfRange { tag: 11 });
            }
            self.raw(11, &h)?.raw(12, &v)
        }

        /// Appends a target to the VTarget Series.
        pub fn target(&mut self, t: &TargetBuilder) -> Result<&mut Self, Error> {
            let pack = t.pack()?;
            enc::push_ber_length(&mut self.series, pack.len())?;
            self.series.extend_from_slice(&pack);
            self.targets += 1;
            Ok(self)
        }

        /// The set body, for nesting as ST 0601 tag 74. Adds the version,
        /// the target counts and the VTarget Series.
        pub fn body(&self) -> Vec<u8> {
            let mut b = self.header.clone();
            enc::push_item(&mut b, 4, &enc::min_uint(VERSION)).expect("small item");
            enc::push_item(&mut b, 5, &enc::min_uint(self.targets)).expect("small item");
            enc::push_item(&mut b, 6, &enc::min_uint(self.targets)).expect("small item");
            if !self.series.is_empty() {
                enc::push_item(&mut b, 101, &self.series).expect("series under 4 GiB");
            }
            b
        }

        /// A standalone packet: key, length, body, checksum.
        pub fn finish(&self) -> Vec<u8> {
            enc::finish_with_checksum(&KEY, &self.body()).expect("packet under 4 GiB")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_numbers() {
        assert_eq!(pixel_to_row_col(1, 1280), Some((1, 1)));
        assert_eq!(pixel_to_row_col(1280, 1280), Some((1, 1280)));
        assert_eq!(pixel_to_row_col(1281, 1280), Some((2, 1)));
        assert_eq!(pixel_to_row_col(0, 1280), None);
    }

    #[test]
    fn hostile_series_does_not_panic() {
        let body = [101u8, 4, 0x84, 0xFF, 0xFF, 0xFF];
        let set = VmtiSet::new(&body);
        let mut t = set.targets();
        assert!(t.next().unwrap().is_err());
        assert!(t.next().is_none());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn standalone_round_trip() {
        let mut b = Builder::new(42);
        b.system_name("unit")
            .unwrap()
            .frame_size(640, 480)
            .unwrap()
            .fov(30.0, 22.5)
            .unwrap()
            .frame_number(9)
            .unwrap();
        let mut t = TargetBuilder::new(300);
        t.centroid_pixel(640 * 10 + 5)
            .unwrap()
            .bounding_box(1, 640 * 20)
            .unwrap()
            .priority(1)
            .unwrap();
        t.location_offset(0.001, -0.002)
            .unwrap()
            .history(12)
            .unwrap();
        b.target(&t).unwrap();
        b.target(&TargetBuilder::new(5)).unwrap();
        let bytes = b.finish();
        let (set, used) = VmtiSet::parse(&bytes).unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(set.timestamp(), Some(42));
        assert_eq!(set.frame_width(), Some(640));
        assert_eq!(set.targets_reported(), Some(2));
        assert_eq!(set.version(), Some(VERSION));
        assert!((set.horizontal_fov().unwrap() - 30.0).abs() < 0.01);
        let ts: Result<alloc::vec::Vec<_>, _> = set.targets().collect();
        let ts = ts.unwrap();
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].id, 300);
        assert_eq!(ts[0].centroid_row_col(640), Some((11, 5)));
        let fc = GeoPoint {
            lat: 10.0,
            lon: 20.0,
            height: None,
        };
        let g = ts[0].ground_position(Some(fc)).unwrap();
        assert!((g.lat - 10.001).abs() < 1e-5 && (g.lon - 19.998).abs() < 1e-5);
        assert_eq!(ts[1].id, 5);
        assert_eq!(ts[1].centroid_pixel(), None);
    }
}
