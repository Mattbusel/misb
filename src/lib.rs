//! MISB KLV metadata, the telemetry carried next to the video in STANAG 4609
//! full motion video from drones and other airborne sensors.
//!
//! | Standard | Module | What it is |
//! |---|---|---|
//! | SMPTE ST 336, MISB ST 0107 | [`klv`] | Keys, BER lengths, BER-OID tags, local sets, splitting a byte stream into packets |
//! | MISB ST 0601 | [`st0601`] | UAS Datalink Local Set: platform, sensor and frame-center position, footprint, timestamps, 143 tags |
//! | MISB ST 0102 | [`st0102`] | Security Local Set (classification markings), nested in ST 0601 tag 48 |
//! | MISB ST 0903 | [`st0903`] | Video Moving Target Indicator (VMTI): detections in the frame, nested in tag 74 or standalone |
//! | MISB ST 1201 | [`imapb`] | IMAPB floating point to integer mapping used by the newer tags |
//!
//! Decoding is zero-copy and does not allocate: a [`st0601::Packet`] borrows
//! the input and hands out typed values on demand. The checksum is verified
//! by [`st0601::Packet::parse`]; lengths are bounded by the input, so
//! corrupt or hostile data returns an [`Error`] instead of panicking.
//! Encoding (the `Builder`s) needs the `alloc` feature, which `std` (the
//! default) turns on.
//!
//! ```
//! # fn main() -> Result<(), misb::Error> {
//! use misb::st0601::{Builder, Packet, Tag};
//!
//! // Build a packet the way a payload computer would ...
//! let mut b = Builder::new(1_700_000_000_000_000);
//! b.text(Tag::MissionId, "TRAINING 4")?
//!     .float(Tag::SensorLatitude, 38.8895)?
//!     .float(Tag::SensorLongitude, -77.0353)?
//!     .float(Tag::SensorTrueAltitude, 450.0)?
//!     .float(Tag::PlatformHeadingAngle, 92.5)?;
//! let bytes = b.finish();
//!
//! // ... and read it back the way a ground station would.
//! let (packet, used) = Packet::parse(&bytes)?;
//! assert_eq!(used, bytes.len());
//! let pos = packet.sensor_position().unwrap();
//! assert!((pos.lat - 38.8895).abs() < 1e-6);
//! assert_eq!(packet.text(Tag::MissionId), Some("TRAINING 4"));
//! # Ok(()) }
//! ```
//!
//! Items that describe weapons (ST 0601 tags 60, 61, 62 and 140) are passed
//! through as raw bytes and never interpreted. This crate covers metadata
//! for situational awareness and interoperability only.
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod imapb;
pub mod klv;
mod math;
pub mod st0102;
pub mod st0601;
pub mod st0903;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

#[cfg(doctest)]
#[doc = include_str!("../docs/integration.md")]
struct IntegrationDoctests;

use core::fmt;

/// Everything that can go wrong while decoding or encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The input ended in the middle of a key, length or value.
    Truncated,
    /// A BER length or BER-OID tag is malformed (too long, or longer than the input).
    BadLength,
    /// The 16-byte key is not the one this decoder expects.
    WrongKey,
    /// The packet does not end with a checksum item (tag 1, two bytes).
    MissingChecksum,
    /// The checksum item does not match the packet.
    Checksum {
        /// Checksum found in the packet.
        found: u16,
        /// Checksum computed over the packet.
        computed: u16,
    },
    /// A value has the wrong length or type for its tag (encoding).
    BadValue {
        /// The tag being encoded.
        tag: u32,
    },
    /// A number is outside the range its tag can carry (encoding).
    OutOfRange {
        /// The tag being encoded.
        tag: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => f.write_str("KLV data ends early"),
            Error::BadLength => f.write_str("malformed BER length or tag"),
            Error::WrongKey => f.write_str("unexpected universal key"),
            Error::MissingChecksum => f.write_str("packet does not end with a checksum item"),
            Error::Checksum { found, computed } => {
                write!(
                    f,
                    "checksum mismatch: packet says {found:#06x}, computed {computed:#06x}"
                )
            }
            Error::BadValue { tag } => write!(f, "tag {tag}: value has the wrong type or length"),
            Error::OutOfRange { tag } => {
                write!(f, "tag {tag}: value outside the range the tag can carry")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// A WGS84 position: latitude and longitude in degrees, height in metres
/// (above mean sea level or the ellipsoid, as the source tag says).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoPoint {
    /// Latitude, degrees north.
    pub lat: f64,
    /// Longitude, degrees east.
    pub lon: f64,
    /// Height in metres, if the packet has it.
    pub height: Option<f64>,
}
