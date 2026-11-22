//! MISB ST 0601 UAS Datalink Local Set: the metadata stream of drone and
//! other airborne full motion video (STANAG 4609).
//!
//! A packet is the 16-byte key [`KEY`], a BER length, then items of the
//! form tag (BER-OID), length (BER), value. The last item is the 16-bit
//! checksum (tag 1). [`Packet::parse`] checks key, lengths and checksum and
//! then reads values lazily; [`tag_info`] holds the mapping of every tag in
//! ST 0601.19 (names, units, how the value is stored).
//!
//! ```
//! use misb::{klv, st0601::{Packet, Tag}};
//!
//! # let stream = misb::st0601::Builder::new(1).finish();
//! for unit in klv::split(&stream) {
//!     let Ok(packet) = Packet::from_unit(&unit) else { continue };
//!     if let (Some(t), Some(fc)) = (packet.timestamp(), packet.frame_center()) {
//!         println!("{t}: looking at {:.5}, {:.5}", fc.lat, fc.lon);
//!     }
//!     for item in packet.items().flatten() {
//!         println!("{:>3} {:<40} {:?}", item.tag, item.name(), item.value());
//!     }
//! }
//! ```

use crate::klv::{self, be_int, be_uint, Items, RawItem};
use crate::{st0102, st0903, Error, GeoPoint};

pub mod packs;

/// The universal key of the UAS Datalink Local Set.
pub const KEY: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
];

/// The ST 0601 revision this crate implements, written to tag 65 by [`Builder`].
pub const VERSION: u64 = 19;

/// How a tag's value is stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Text (ISO 646 / UTF-8).
    Text,
    /// Microseconds since 1970-01-01 UTC (MISP time), unsigned, 8 bytes for
    /// tag 2 and up to 8 for the others.
    Time,
    /// Unsigned integer, 1 to 8 bytes.
    Uint,
    /// Signed integer, 1 to 8 bytes.
    Int,
    /// ST 0601 fixed linear mapping onto an integer of `len` bytes. For a
    /// signed mapping the most negative integer means "out of range".
    Linear {
        /// Byte length.
        len: u8,
        /// Two's complement integer.
        signed: bool,
        /// Lowest value.
        min: f64,
        /// Highest value.
        max: f64,
    },
    /// ST 1201 IMAPB over `min..=max`; any length decodes, `len` is what
    /// [`Builder::float`] writes.
    Imapb {
        /// Lowest value.
        min: f64,
        /// Highest value.
        max: f64,
        /// Default byte length when encoding.
        len: u8,
    },
    /// MISB ST 0102 Security Local Set.
    Security,
    /// MISB ST 0903 VMTI Local Set.
    Vmti,
    /// Packs, lists, other nested sets and deliberately uninterpreted items.
    Bytes,
}

/// Name, units and storage of one tag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TagInfo {
    /// The tag.
    pub tag: Tag,
    /// The name used in ST 0601.
    pub name: &'static str,
    /// Units of the decoded value (`""` when not applicable).
    pub units: &'static str,
    /// How the value is stored.
    pub kind: Kind,
}

const fn lin(len: u8, signed: bool, min: f64, max: f64) -> Kind {
    Kind::Linear {
        len,
        signed,
        min,
        max,
    }
}
const fn imapb(min: f64, max: f64, len: u8) -> Kind {
    Kind::Imapb { min, max, len }
}

macro_rules! tags {
    ($( $num:literal $variant:ident $name:literal [$units:literal] $kind:expr; )*) => {
        /// ST 0601 tags (ST 0601.19).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        #[repr(u8)]
        pub enum Tag {
            $( #[doc = $name] $variant = $num, )*
        }

        impl Tag {
            /// The tag with number `n`, if ST 0601.19 defines it.
            pub const fn from_u32(n: u32) -> Option<Tag> {
                match n {
                    $( $num => Some(Tag::$variant), )*
                    _ => None,
                }
            }
        }

        /// Every ST 0601.19 tag, in tag order.
        pub static TAGS: &[TagInfo] = &[
            $( TagInfo { tag: Tag::$variant, name: $name, units: $units, kind: $kind }, )*
        ];
    };
}

tags! {
    1 Checksum "Checksum" [""] Kind::Uint;
    2 PrecisionTimeStamp "Precision Time Stamp" ["us"] Kind::Time;
    3 MissionId "Mission ID" [""] Kind::Text;
    4 PlatformTailNumber "Platform Tail Number" [""] Kind::Text;
    5 PlatformHeadingAngle "Platform Heading Angle" ["deg"] lin(2, false, 0.0, 360.0);
    6 PlatformPitchAngle "Platform Pitch Angle" ["deg"] lin(2, true, -20.0, 20.0);
    7 PlatformRollAngle "Platform Roll Angle" ["deg"] lin(2, true, -50.0, 50.0);
    8 PlatformTrueAirspeed "Platform True Airspeed" ["m/s"] Kind::Uint;
    9 PlatformIndicatedAirspeed "Platform Indicated Airspeed" ["m/s"] Kind::Uint;
    10 PlatformDesignation "Platform Designation" [""] Kind::Text;
    11 ImageSourceSensor "Image Source Sensor" [""] Kind::Text;
    12 ImageCoordinateSystem "Image Coordinate System" [""] Kind::Text;
    13 SensorLatitude "Sensor Latitude" ["deg"] lin(4, true, -90.0, 90.0);
    14 SensorLongitude "Sensor Longitude" ["deg"] lin(4, true, -180.0, 180.0);
    15 SensorTrueAltitude "Sensor True Altitude" ["m"] lin(2, false, -900.0, 19000.0);
    16 SensorHorizontalFieldOfView "Sensor Horizontal Field of View" ["deg"] lin(2, false, 0.0, 180.0);
    17 SensorVerticalFieldOfView "Sensor Vertical Field of View" ["deg"] lin(2, false, 0.0, 180.0);
    18 SensorRelativeAzimuthAngle "Sensor Relative Azimuth Angle" ["deg"] lin(4, false, 0.0, 360.0);
    19 SensorRelativeElevationAngle "Sensor Relative Elevation Angle" ["deg"] lin(4, true, -180.0, 180.0);
    20 SensorRelativeRollAngle "Sensor Relative Roll Angle" ["deg"] lin(4, false, 0.0, 360.0);
    21 SlantRange "Slant Range" ["m"] lin(4, false, 0.0, 5_000_000.0);
    22 TargetWidth "Target Width" ["m"] lin(2, false, 0.0, 10_000.0);
    23 FrameCenterLatitude "Frame Center Latitude" ["deg"] lin(4, true, -90.0, 90.0);
    24 FrameCenterLongitude "Frame Center Longitude" ["deg"] lin(4, true, -180.0, 180.0);
    25 FrameCenterElevation "Frame Center Elevation" ["m"] lin(2, false, -900.0, 19000.0);
    26 OffsetCornerLatitudePoint1 "Offset Corner Latitude Point 1" ["deg"] lin(2, true, -0.075, 0.075);
    27 OffsetCornerLongitudePoint1 "Offset Corner Longitude Point 1" ["deg"] lin(2, true, -0.075, 0.075);
    28 OffsetCornerLatitudePoint2 "Offset Corner Latitude Point 2" ["deg"] lin(2, true, -0.075, 0.075);
    29 OffsetCornerLongitudePoint2 "Offset Corner Longitude Point 2" ["deg"] lin(2, true, -0.075, 0.075);
    30 OffsetCornerLatitudePoint3 "Offset Corner Latitude Point 3" ["deg"] lin(2, true, -0.075, 0.075);
    31 OffsetCornerLongitudePoint3 "Offset Corner Longitude Point 3" ["deg"] lin(2, true, -0.075, 0.075);
    32 OffsetCornerLatitudePoint4 "Offset Corner Latitude Point 4" ["deg"] lin(2, true, -0.075, 0.075);
    33 OffsetCornerLongitudePoint4 "Offset Corner Longitude Point 4" ["deg"] lin(2, true, -0.075, 0.075);
    34 IcingDetected "Icing Detected" [""] Kind::Uint;
    35 WindDirection "Wind Direction" ["deg"] lin(2, false, 0.0, 360.0);
    36 WindSpeed "Wind Speed" ["m/s"] lin(1, false, 0.0, 100.0);
    37 StaticPressure "Static Pressure" ["mbar"] lin(2, false, 0.0, 5000.0);
    38 DensityAltitude "Density Altitude" ["m"] lin(2, false, -900.0, 19000.0);
    39 OutsideAirTemperature "Outside Air Temperature" ["C"] Kind::Int;
    40 TargetLocationLatitude "Target Location Latitude" ["deg"] lin(4, true, -90.0, 90.0);
    41 TargetLocationLongitude "Target Location Longitude" ["deg"] lin(4, true, -180.0, 180.0);
    42 TargetLocationElevation "Target Location Elevation" ["m"] lin(2, false, -900.0, 19000.0);
    43 TargetTrackGateWidth "Target Track Gate Width" ["pixels"] lin(1, false, 0.0, 510.0);
    44 TargetTrackGateHeight "Target Track Gate Height" ["pixels"] lin(1, false, 0.0, 510.0);
    45 TargetErrorEstimateCe90 "Target Error Estimate - CE90" ["m"] lin(2, false, 0.0, 4095.0);
    46 TargetErrorEstimateLe90 "Target Error Estimate - LE90" ["m"] lin(2, false, 0.0, 4095.0);
    47 GenericFlagData "Generic Flag Data" [""] Kind::Uint;
    48 SecurityLocalSet "Security Local Set" [""] Kind::Security;
    49 DifferentialPressure "Differential Pressure" ["mbar"] lin(2, false, 0.0, 5000.0);
    50 PlatformAngleOfAttack "Platform Angle of Attack" ["deg"] lin(2, true, -20.0, 20.0);
    51 PlatformVerticalSpeed "Platform Vertical Speed" ["m/s"] lin(2, true, -180.0, 180.0);
    52 PlatformSideslipAngle "Platform Sideslip Angle" ["deg"] lin(2, true, -20.0, 20.0);
    53 AirfieldBarometricPressure "Airfield Barometric Pressure" ["mbar"] lin(2, false, 0.0, 5000.0);
    54 AirfieldElevation "Airfield Elevation" ["m"] lin(2, false, -900.0, 19000.0);
    55 RelativeHumidity "Relative Humidity" ["%"] lin(1, false, 0.0, 100.0);
    56 PlatformGroundSpeed "Platform Ground Speed" ["m/s"] Kind::Uint;
    57 GroundRange "Ground Range" ["m"] lin(4, false, 0.0, 5_000_000.0);
    58 PlatformFuelRemaining "Platform Fuel Remaining" ["kg"] lin(2, false, 0.0, 10_000.0);
    59 PlatformCallSign "Platform Call Sign" [""] Kind::Text;
    60 Tag60 "Weapon Load (not interpreted)" [""] Kind::Bytes;
    61 Tag61 "Weapon Fired (not interpreted)" [""] Kind::Bytes;
    62 Tag62 "Laser PRF Code (not interpreted)" [""] Kind::Bytes;
    63 SensorFieldOfViewName "Sensor Field of View Name" [""] Kind::Uint;
    64 PlatformMagneticHeading "Platform Magnetic Heading" ["deg"] lin(2, false, 0.0, 360.0);
    65 UasLsVersionNumber "UAS Datalink LS Version Number" [""] Kind::Uint;
    66 Deprecated66 "Deprecated (Target Location Covariance Matrix)" [""] Kind::Bytes;
    67 AlternatePlatformLatitude "Alternate Platform Latitude" ["deg"] lin(4, true, -90.0, 90.0);
    68 AlternatePlatformLongitude "Alternate Platform Longitude" ["deg"] lin(4, true, -180.0, 180.0);
    69 AlternatePlatformAltitude "Alternate Platform Altitude" ["m"] lin(2, false, -900.0, 19000.0);
    70 AlternatePlatformName "Alternate Platform Name" [""] Kind::Text;
    71 AlternatePlatformHeading "Alternate Platform Heading" ["deg"] lin(2, false, 0.0, 360.0);
    72 EventStartTime "Event Start Time - UTC" ["us"] Kind::Time;
    73 RvtLocalSet "RVT Local Set (ST 0806)" [""] Kind::Bytes;
    74 VmtiLocalSet "VMTI Local Set (ST 0903)" [""] Kind::Vmti;
    75 SensorEllipsoidHeight "Sensor Ellipsoid Height" ["m"] lin(2, false, -900.0, 19000.0);
    76 AlternatePlatformEllipsoidHeight "Alternate Platform Ellipsoid Height" ["m"] lin(2, false, -900.0, 19000.0);
    77 OperationalMode "Operational Mode" [""] Kind::Uint;
    78 FrameCenterHeightAboveEllipsoid "Frame Center Height Above Ellipsoid" ["m"] lin(2, false, -900.0, 19000.0);
    79 SensorNorthVelocity "Sensor North Velocity" ["m/s"] lin(2, true, -327.0, 327.0);
    80 SensorEastVelocity "Sensor East Velocity" ["m/s"] lin(2, true, -327.0, 327.0);
    81 ImageHorizonPixelPack "Image Horizon Pixel Pack" [""] Kind::Bytes;
    82 CornerLatitudePoint1Full "Corner Latitude Point 1 (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    83 CornerLongitudePoint1Full "Corner Longitude Point 1 (Full)" ["deg"] lin(4, true, -180.0, 180.0);
    84 CornerLatitudePoint2Full "Corner Latitude Point 2 (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    85 CornerLongitudePoint2Full "Corner Longitude Point 2 (Full)" ["deg"] lin(4, true, -180.0, 180.0);
    86 CornerLatitudePoint3Full "Corner Latitude Point 3 (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    87 CornerLongitudePoint3Full "Corner Longitude Point 3 (Full)" ["deg"] lin(4, true, -180.0, 180.0);
    88 CornerLatitudePoint4Full "Corner Latitude Point 4 (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    89 CornerLongitudePoint4Full "Corner Longitude Point 4 (Full)" ["deg"] lin(4, true, -180.0, 180.0);
    90 PlatformPitchAngleFull "Platform Pitch Angle (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    91 PlatformRollAngleFull "Platform Roll Angle (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    92 PlatformAngleOfAttackFull "Platform Angle of Attack (Full)" ["deg"] lin(4, true, -90.0, 90.0);
    93 PlatformSideslipAngleFull "Platform Sideslip Angle (Full)" ["deg"] lin(4, true, -180.0, 180.0);
    94 MiisCoreIdentifier "MIIS Core Identifier (ST 1204)" [""] Kind::Bytes;
    95 SarMotionImageryLocalSet "SAR Motion Imagery Local Set (ST 1206)" [""] Kind::Bytes;
    96 TargetWidthExtended "Target Width Extended" ["m"] imapb(0.0, 1_500_000.0, 3);
    97 RangeImageLocalSet "Range Image Local Set (ST 1002)" [""] Kind::Bytes;
    98 GeoRegistrationLocalSet "Geo-Registration Local Set (ST 1601)" [""] Kind::Bytes;
    99 CompositeImagingLocalSet "Composite Imaging Local Set (ST 1602)" [""] Kind::Bytes;
    100 SegmentLocalSet "Segment Local Set (ST 1607)" [""] Kind::Bytes;
    101 AmendLocalSet "Amend Local Set (ST 1607)" [""] Kind::Bytes;
    102 SdccFlp "SDCC-FLP (ST 1010)" [""] Kind::Bytes;
    103 DensityAltitudeExtended "Density Altitude Extended" ["m"] imapb(-900.0, 40_000.0, 3);
    104 SensorEllipsoidHeightExtended "Sensor Ellipsoid Height Extended" ["m"] imapb(-900.0, 40_000.0, 3);
    105 AlternatePlatformEllipsoidHeightExtended "Alternate Platform Ellipsoid Height Extended" ["m"] imapb(-900.0, 40_000.0, 3);
    106 StreamDesignator "Stream Designator" [""] Kind::Text;
    107 OperationalBase "Operational Base" [""] Kind::Text;
    108 BroadcastSource "Broadcast Source" [""] Kind::Text;
    109 RangeToRecoveryLocation "Range To Recovery Location" ["km"] imapb(0.0, 21_000.0, 3);
    110 TimeAirborne "Time Airborne" ["s"] Kind::Uint;
    111 PropulsionUnitSpeed "Propulsion Unit Speed" ["rpm"] Kind::Uint;
    112 PlatformCourseAngle "Platform Course Angle" ["deg"] imapb(0.0, 360.0, 2);
    113 AltitudeAgl "Altitude Above Ground Level (AGL)" ["m"] imapb(-900.0, 40_000.0, 3);
    114 RadarAltimeter "Radar Altimeter" ["m"] imapb(-900.0, 40_000.0, 3);
    115 ControlCommand "Control Command" [""] Kind::Bytes;
    116 ControlCommandVerificationList "Control Command Verification List" [""] Kind::Bytes;
    117 SensorAzimuthRate "Sensor Azimuth Rate" ["deg/s"] imapb(-1000.0, 1000.0, 3);
    118 SensorElevationRate "Sensor Elevation Rate" ["deg/s"] imapb(-1000.0, 1000.0, 3);
    119 SensorRollRate "Sensor Roll Rate" ["deg/s"] imapb(-1000.0, 1000.0, 3);
    120 OnboardMiStoragePercentFull "On-board MI Storage Percent Full" ["%"] imapb(0.0, 100.0, 2);
    121 ActiveWavelengthList "Active Wavelength List" [""] Kind::Bytes;
    122 CountryCodes "Country Codes" [""] Kind::Bytes;
    123 NumberOfNavsatsInView "Number of NAVSATs in View" [""] Kind::Uint;
    124 PositioningMethodSource "Positioning Method Source" [""] Kind::Uint;
    125 PlatformStatus "Platform Status" [""] Kind::Uint;
    126 SensorControlMode "Sensor Control Mode" [""] Kind::Uint;
    127 SensorFrameRatePack "Sensor Frame Rate Pack" [""] Kind::Bytes;
    128 WavelengthsList "Wavelengths List" [""] Kind::Bytes;
    129 TargetId "Target ID" [""] Kind::Text;
    130 AirbaseLocations "Airbase Locations" [""] Kind::Bytes;
    131 TakeOffTime "Take-off Time" ["us"] Kind::Time;
    132 TransmissionFrequency "Transmission Frequency" ["MHz"] imapb(1.0, 99_999.0, 3);
    133 OnboardMiStorageCapacity "On-board MI Storage Capacity" ["GB"] Kind::Uint;
    134 ZoomPercentage "Zoom Percentage" ["%"] imapb(0.0, 100.0, 2);
    135 CommunicationsMethod "Communications Method" [""] Kind::Text;
    136 LeapSeconds "Leap Seconds" ["s"] Kind::Int;
    137 CorrectionOffset "Correction Offset" ["us"] Kind::Int;
    138 PayloadList "Payload List" [""] Kind::Bytes;
    139 ActivePayloads "Active Payloads" [""] Kind::Bytes;
    140 Tag140 "Weapons Stores (not interpreted)" [""] Kind::Bytes;
    141 WaypointList "Waypoint List" [""] Kind::Bytes;
    142 ViewDomain "View Domain" [""] Kind::Bytes;
    143 MetadataSubstreamIdPack "Metadata Substream ID Pack" [""] Kind::Bytes;
}

/// Name, units and storage of tag `n`, or `None` for tags ST 0601.19 does not define.
pub fn tag_info(n: u32) -> Option<&'static TagInfo> {
    let i = n.checked_sub(1)? as usize;
    TAGS.get(i).filter(|t| t.tag as u32 == n)
}

impl Tag {
    /// Name, units and storage of this tag.
    pub fn info(self) -> &'static TagInfo {
        &TAGS[self as usize - 1]
    }
}

/// A decoded value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    /// A mapped number in the tag's units.
    Float(f64),
    /// An unsigned integer (counts, enumerations, flags).
    Uint(u64),
    /// A signed integer.
    Int(i64),
    /// Text.
    Text(&'a str),
    /// Microseconds since 1970-01-01 UTC.
    Time(u64),
    /// Nested ST 0102 Security Local Set.
    Security(st0102::SecuritySet<'a>),
    /// Nested ST 0903 VMTI Local Set.
    Vmti(st0903::VmtiSet<'a>),
    /// Raw bytes (packs, other nested sets, uninterpreted and unknown tags).
    Bytes(&'a [u8]),
    /// The sender marked the value as out of range or not available.
    OutOfRange,
    /// The value does not fit its tag (wrong length, not text).
    Invalid,
}

impl<'a> Value<'a> {
    /// The value as a number, if it is one.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Float(v) => Some(v),
            Value::Uint(v) | Value::Time(v) => Some(v as f64),
            Value::Int(v) => Some(v as f64),
            _ => None,
        }
    }
}

/// Decodes the value of tag `tag`: [`decode_value`] with the tag's kind,
/// [`Value::Bytes`] for tags ST 0601.19 does not define, and
/// [`Value::Invalid`] for a Precision Time Stamp (tag 2) that is not 8 bytes.
pub fn decode_tag(tag: u32, raw: &[u8]) -> Value<'_> {
    match tag_info(tag) {
        Some(_) if tag == Tag::PrecisionTimeStamp as u32 && raw.len() != 8 => Value::Invalid,
        Some(t) => decode_value(t.kind, raw),
        None => Value::Bytes(raw),
    }
}

/// Decodes `raw` according to `kind`.
pub fn decode_value(kind: Kind, raw: &[u8]) -> Value<'_> {
    match kind {
        Kind::Text => core::str::from_utf8(raw)
            .map(Value::Text)
            .unwrap_or(Value::Invalid),
        // Tag 2 is always 8 bytes; newer time tags (Take-off Time, 131) may
        // be sent shorter, as KWIVER does. [`crate::conform`] flags a short tag 2.
        Kind::Time => be_uint(raw).map(Value::Time).unwrap_or(Value::Invalid),
        Kind::Uint => be_uint(raw).map(Value::Uint).unwrap_or(Value::Invalid),
        Kind::Int => be_int(raw).map(Value::Int).unwrap_or(Value::Invalid),
        Kind::Linear {
            len,
            signed,
            min,
            max,
        } => {
            // Some encoders write a mapped tag in more or fewer bytes than the
            // standard says (tag 22 in four bytes is common). Like klvdata, read
            // the integer as sent and apply the standard scale.
            if raw.is_empty() || raw.len() > 8 {
                return Value::Invalid;
            }
            let bits = 8 * len as i32;
            if signed {
                let v = be_int(raw).unwrap_or(0);
                if raw.len() == len as usize && v == -(1i64 << (bits - 1)) {
                    return Value::OutOfRange;
                }
                let span = crate::math::pow2(bits) - 2.0;
                Value::Float(v as f64 * (max - min) / span + (max + min) / 2.0)
            } else {
                let v = be_uint(raw).unwrap_or(0);
                let span = crate::math::pow2(bits) - 1.0;
                Value::Float(v as f64 * (max - min) / span + min)
            }
        }
        Kind::Imapb { min, max, .. } => {
            if raw.is_empty() || raw.len() > 8 {
                return Value::Invalid;
            }
            crate::imapb::decode(min, max, raw)
                .map(Value::Float)
                .unwrap_or(Value::OutOfRange)
        }
        Kind::Security => Value::Security(st0102::SecuritySet::new(raw)),
        Kind::Vmti => Value::Vmti(st0903::VmtiSet::new(raw)),
        Kind::Bytes => Value::Bytes(raw),
    }
}

/// One item of a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item<'a> {
    /// Tag number.
    pub tag: u32,
    /// Raw value bytes.
    pub raw: &'a [u8],
}

impl<'a> Item<'a> {
    /// The tag, if ST 0601.19 defines it.
    pub fn known_tag(&self) -> Option<Tag> {
        Tag::from_u32(self.tag)
    }

    /// The tag's ST 0601 name, or `"Unknown"`.
    pub fn name(&self) -> &'static str {
        tag_info(self.tag).map_or("Unknown", |t| t.name)
    }

    /// The decoded value; unknown tags come back as [`Value::Bytes`].
    pub fn value(&self) -> Value<'a> {
        decode_tag(self.tag, self.raw)
    }
}

/// A UAS Datalink Local Set packet, borrowed from the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    body: &'a [u8],
}

impl<'a> Packet<'a> {
    /// Parses a packet at the start of `buf`: checks the key, the length and
    /// the checksum. Returns the packet and the number of bytes it used.
    pub fn parse(buf: &'a [u8]) -> Result<(Packet<'a>, usize), Error> {
        let (body, used) = klv::parse_checked(buf, &KEY, true)?;
        Ok((Packet { body }, used))
    }

    /// Like [`Packet::parse`] but skips the checksum (for captures where the
    /// sender is known to compute it wrongly).
    pub fn parse_unverified(buf: &'a [u8]) -> Result<(Packet<'a>, usize), Error> {
        let (body, used) = klv::parse_checked(buf, &KEY, false)?;
        Ok((Packet { body }, used))
    }

    /// Checks a unit found by [`klv::split`].
    pub fn from_unit(unit: &klv::Unit<'a>) -> Result<Packet<'a>, Error> {
        Packet::parse(unit.raw).map(|(p, _)| p)
    }

    /// Wraps a local set body that has already been separated from its key
    /// (for example the payload of a VMTI or video container field). No
    /// checksum check is possible.
    pub fn from_body(body: &'a [u8]) -> Packet<'a> {
        Packet { body }
    }

    /// The local set body (all items, checksum included).
    pub fn body(&self) -> &'a [u8] {
        self.body
    }

    /// All items in packet order.
    pub fn items(&self) -> impl Iterator<Item = Result<Item<'a>, Error>> + 'a {
        Items::new(self.body).map(|r| r.map(|RawItem { tag, value }| Item { tag, raw: value }))
    }

    /// Raw bytes of the first item with tag `tag`.
    pub fn raw(&self, tag: Tag) -> Option<&'a [u8]> {
        klv::find(self.body, tag as u32)
    }

    /// Decoded value of `tag`, if present.
    pub fn value(&self, tag: Tag) -> Option<Value<'a>> {
        self.raw(tag).map(|r| decode_tag(tag as u32, r))
    }

    /// `tag` as a number in its units; `None` if missing or out of range.
    pub fn float(&self, tag: Tag) -> Option<f64> {
        self.value(tag)?.as_f64()
    }

    /// `tag` as text.
    pub fn text(&self, tag: Tag) -> Option<&'a str> {
        match self.value(tag)? {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Precision Time Stamp (tag 2), microseconds since 1970 UTC.
    pub fn timestamp(&self) -> Option<u64> {
        if self.raw(Tag::PrecisionTimeStamp)?.len() != 8 {
            return None;
        }
        match self.value(Tag::PrecisionTimeStamp)? {
            Value::Time(t) => Some(t),
            _ => None,
        }
    }

    /// UAS Datalink LS version (tag 65).
    pub fn version(&self) -> Option<u64> {
        match self.value(Tag::UasLsVersionNumber)? {
            Value::Uint(v) => Some(v),
            _ => None,
        }
    }

    fn point(&self, lat: Tag, lon: Tag, h: Tag) -> Option<GeoPoint> {
        Some(GeoPoint {
            lat: self.float(lat)?,
            lon: self.float(lon)?,
            height: self.float(h),
        })
    }

    /// Sensor position (tags 13, 14; height is tag 15, metres MSL).
    pub fn sensor_position(&self) -> Option<GeoPoint> {
        self.point(
            Tag::SensorLatitude,
            Tag::SensorLongitude,
            Tag::SensorTrueAltitude,
        )
    }

    /// Sensor height above the WGS84 ellipsoid (tag 104, else tag 75).
    pub fn sensor_hae(&self) -> Option<f64> {
        self.float(Tag::SensorEllipsoidHeightExtended)
            .or_else(|| self.float(Tag::SensorEllipsoidHeight))
    }

    /// Center of the image on the ground (tags 23, 24; height is tag 25, metres MSL).
    pub fn frame_center(&self) -> Option<GeoPoint> {
        self.point(
            Tag::FrameCenterLatitude,
            Tag::FrameCenterLongitude,
            Tag::FrameCenterElevation,
        )
    }

    /// Target location (tags 40, 41, 42), the point the sensor is tracking.
    pub fn target_location(&self) -> Option<GeoPoint> {
        self.point(
            Tag::TargetLocationLatitude,
            Tag::TargetLocationLongitude,
            Tag::TargetLocationElevation,
        )
    }

    /// Alternate platform position (tags 67, 68, 69).
    pub fn alternate_platform_position(&self) -> Option<GeoPoint> {
        self.point(
            Tag::AlternatePlatformLatitude,
            Tag::AlternatePlatformLongitude,
            Tag::AlternatePlatformAltitude,
        )
    }

    /// The four image corners on the ground (upper left, upper right, lower
    /// right, lower left): the full-precision tags 82 to 89 when all are
    /// present, otherwise the offset tags 26 to 33 added to the frame center.
    pub fn corners(&self) -> Option<[GeoPoint; 4]> {
        use Tag::*;
        let full = [
            (CornerLatitudePoint1Full, CornerLongitudePoint1Full),
            (CornerLatitudePoint2Full, CornerLongitudePoint2Full),
            (CornerLatitudePoint3Full, CornerLongitudePoint3Full),
            (CornerLatitudePoint4Full, CornerLongitudePoint4Full),
        ];
        let offs = [
            (OffsetCornerLatitudePoint1, OffsetCornerLongitudePoint1),
            (OffsetCornerLatitudePoint2, OffsetCornerLongitudePoint2),
            (OffsetCornerLatitudePoint3, OffsetCornerLongitudePoint3),
            (OffsetCornerLatitudePoint4, OffsetCornerLongitudePoint4),
        ];
        let mut out = [GeoPoint {
            lat: 0.0,
            lon: 0.0,
            height: None,
        }; 4];
        let got_full = full.iter().zip(out.iter_mut()).all(|(&(la, lo), o)| {
            match (self.float(la), self.float(lo)) {
                (Some(lat), Some(lon)) => {
                    *o = GeoPoint {
                        lat,
                        lon,
                        height: None,
                    };
                    true
                }
                _ => false,
            }
        });
        if got_full {
            return Some(out);
        }
        let fc = self.frame_center()?;
        for (&(la, lo), o) in offs.iter().zip(out.iter_mut()) {
            *o = GeoPoint {
                lat: fc.lat + self.float(la)?,
                lon: fc.lon + self.float(lo)?,
                height: None,
            };
        }
        Some(out)
    }

    /// Platform heading, degrees true (tag 5).
    pub fn platform_heading(&self) -> Option<f64> {
        self.float(Tag::PlatformHeadingAngle)
    }

    /// Platform pitch in degrees: tag 90 (full range) if present, else tag 6.
    pub fn platform_pitch(&self) -> Option<f64> {
        self.float(Tag::PlatformPitchAngleFull)
            .or_else(|| self.float(Tag::PlatformPitchAngle))
    }

    /// Platform roll in degrees: tag 91 (full range) if present, else tag 7.
    pub fn platform_roll(&self) -> Option<f64> {
        self.float(Tag::PlatformRollAngleFull)
            .or_else(|| self.float(Tag::PlatformRollAngle))
    }

    /// The nested ST 0102 Security Local Set (tag 48).
    pub fn security(&self) -> Option<st0102::SecuritySet<'a>> {
        self.raw(Tag::SecurityLocalSet)
            .map(st0102::SecuritySet::new)
    }

    /// The nested ST 0903 VMTI Local Set (tag 74).
    pub fn vmti(&self) -> Option<st0903::VmtiSet<'a>> {
        self.raw(Tag::VmtiLocalSet).map(st0903::VmtiSet::new)
    }

    /// Image Horizon Pixel Pack (tag 81).
    pub fn image_horizon(&self) -> Option<packs::HorizonPixelPack> {
        packs::HorizonPixelPack::parse(self.raw(Tag::ImageHorizonPixelPack)?).ok()
    }

    /// Every Control Command (tag 115) in the packet, in order. Malformed
    /// ones are skipped.
    pub fn control_commands(&self) -> impl Iterator<Item = packs::ControlCommand<'a>> + 'a {
        self.items()
            .map_while(Result::ok)
            .filter(|i| i.tag == Tag::ControlCommand as u32)
            .filter_map(|i| packs::ControlCommand::parse(i.raw).ok())
    }

    /// Ids of the acknowledged control commands (tag 116).
    pub fn verified_commands(&self) -> Option<packs::OidList<'a>> {
        packs::OidList::parse(self.raw(Tag::ControlCommandVerificationList)?).ok()
    }

    /// Ids of the wavelengths in use (tag 121).
    pub fn active_wavelengths(&self) -> Option<packs::OidList<'a>> {
        packs::OidList::parse(self.raw(Tag::ActiveWavelengthList)?).ok()
    }

    /// Country Codes (tag 122).
    pub fn country_codes(&self) -> Option<packs::CountryCodes<'a>> {
        packs::CountryCodes::parse(self.raw(Tag::CountryCodes)?).ok()
    }

    /// Sensor Frame Rate Pack (tag 127).
    pub fn frame_rate(&self) -> Option<packs::FrameRate> {
        packs::FrameRate::parse(self.raw(Tag::SensorFrameRatePack)?).ok()
    }

    /// Wavelengths List (tag 128).
    pub fn wavelengths(&self) -> Option<packs::Wavelengths<'a>> {
        packs::Wavelengths::parse(self.raw(Tag::WavelengthsList)?).ok()
    }

    /// Airbase Locations (tag 130).
    pub fn airbase_locations(&self) -> Option<packs::AirbaseLocations> {
        packs::AirbaseLocations::parse(self.raw(Tag::AirbaseLocations)?).ok()
    }

    /// Payload List (tag 138).
    pub fn payloads(&self) -> Option<packs::PayloadList<'a>> {
        packs::PayloadList::parse(self.raw(Tag::PayloadList)?).ok()
    }

    /// Active Payloads (tag 139).
    pub fn active_payloads(&self) -> Option<packs::ActivePayloads<'a>> {
        self.raw(Tag::ActivePayloads)
            .map(packs::ActivePayloads::new)
    }

    /// Waypoint List (tag 141).
    pub fn waypoints(&self) -> Option<packs::Waypoints<'a>> {
        packs::Waypoints::parse(self.raw(Tag::WaypointList)?).ok()
    }

    /// View Domain (tag 142).
    pub fn view_domain(&self) -> Option<packs::ViewDomain> {
        packs::ViewDomain::parse(self.raw(Tag::ViewDomain)?).ok()
    }
}

#[cfg(feature = "alloc")]
pub use builder::Builder;

#[cfg(feature = "alloc")]
mod builder {
    use super::*;
    use crate::klv::enc;
    use alloc::vec::Vec;

    /// Encodes a UAS Datalink Local Set packet.
    ///
    /// Items are written in the order they are added, after the Precision
    /// Time Stamp. [`Builder::finish`] appends the version (tag 65, unless
    /// set) and the checksum.
    #[derive(Clone, Debug, Default)]
    pub struct Builder {
        body: Vec<u8>,
        has_version: bool,
    }

    impl Builder {
        /// Starts a packet with Precision Time Stamp `timestamp_us`
        /// (microseconds since 1970 UTC).
        pub fn new(timestamp_us: u64) -> Self {
            let mut b = Builder::default();
            enc::push_item(&mut b.body, 2, &timestamp_us.to_be_bytes()).expect("small item");
            b
        }

        /// Adds a raw item. Use this for packs and nested sets this crate
        /// does not model.
        pub fn raw(&mut self, tag: u32, value: &[u8]) -> Result<&mut Self, Error> {
            if tag == 1 {
                return Err(Error::BadValue { tag });
            }
            if tag == Tag::UasLsVersionNumber as u32 {
                self.has_version = true;
            }
            enc::push_item(&mut self.body, tag, value)?;
            Ok(self)
        }

        /// Adds a number in the tag's units. Works for linear, IMAPB and
        /// integer tags; the value must be inside the tag's range.
        pub fn float(&mut self, tag: Tag, value: f64) -> Result<&mut Self, Error> {
            let t = tag as u32;
            match tag.info().kind {
                Kind::Linear {
                    len,
                    signed,
                    min,
                    max,
                } => {
                    if !value.is_finite() || value < min || value > max {
                        return Err(Error::OutOfRange { tag: t });
                    }
                    let bits = 8 * len as i32;
                    let bytes = if signed {
                        let span = crate::math::pow2(bits) - 2.0;
                        let raw =
                            crate::math::round((value - (max + min) / 2.0) * span / (max - min))
                                as i64;
                        raw.to_be_bytes()
                    } else {
                        let span = crate::math::pow2(bits) - 1.0;
                        let raw = crate::math::round((value - min) * span / (max - min)) as u64;
                        raw.to_be_bytes()
                    };
                    self.raw(t, &bytes[8 - len as usize..])
                }
                Kind::Imapb { len, .. } => self.float_len(tag, value, len as usize),
                Kind::Uint
                    if value >= 0.0 && value == crate::math::floor(value) && value < 1.8e19 =>
                {
                    self.uint(tag, value as u64)
                }
                Kind::Int
                    if value == crate::math::floor(value) && value > -9.2e18 && value < 9.2e18 =>
                {
                    self.int(tag, value as i64)
                }
                _ => Err(Error::BadValue { tag: t }),
            }
        }

        /// Adds an IMAPB number with an explicit byte length (1 to 8).
        pub fn float_len(&mut self, tag: Tag, value: f64, len: usize) -> Result<&mut Self, Error> {
            let t = tag as u32;
            let Kind::Imapb { min, max, .. } = tag.info().kind else {
                return Err(Error::BadValue { tag: t });
            };
            if !(1..=8).contains(&len) {
                return Err(Error::BadValue { tag: t });
            }
            let mut b = [0u8; 8];
            if !crate::imapb::Imapb::new(min, max).encode(value, &mut b[..len]) {
                return Err(Error::OutOfRange { tag: t });
            }
            self.raw(t, &b[..len])
        }

        /// Adds an unsigned integer tag (counts, enumerations, flags) in the
        /// fewest bytes.
        pub fn uint(&mut self, tag: Tag, value: u64) -> Result<&mut Self, Error> {
            match tag.info().kind {
                Kind::Uint => self.raw(tag as u32, &enc::min_uint(value)),
                Kind::Time => self.time(tag, value),
                _ => Err(Error::BadValue { tag: tag as u32 }),
            }
        }

        /// Adds a signed integer tag.
        pub fn int(&mut self, tag: Tag, value: i64) -> Result<&mut Self, Error> {
            if tag.info().kind != Kind::Int {
                return Err(Error::BadValue { tag: tag as u32 });
            }
            let b = value.to_be_bytes();
            let skip = (0..7)
                .take_while(|&i| {
                    (b[i] == 0 && b[i + 1] < 0x80) || (b[i] == 0xFF && b[i + 1] >= 0x80)
                })
                .count();
            self.raw(tag as u32, &b[skip..])
        }

        /// Adds a text tag (at most 127 bytes, as ST 0601 requires).
        pub fn text(&mut self, tag: Tag, value: &str) -> Result<&mut Self, Error> {
            if tag.info().kind != Kind::Text || value.len() > 127 {
                return Err(Error::BadValue { tag: tag as u32 });
            }
            self.raw(tag as u32, value.as_bytes())
        }

        /// Adds a time tag (microseconds since 1970 UTC).
        pub fn time(&mut self, tag: Tag, us: u64) -> Result<&mut Self, Error> {
            if tag.info().kind != Kind::Time {
                return Err(Error::BadValue { tag: tag as u32 });
            }
            self.raw(tag as u32, &us.to_be_bytes())
        }

        /// Nests a Security Local Set (tag 48) built with [`st0102::Builder`].
        pub fn security(&mut self, set: &st0102::Builder) -> Result<&mut Self, Error> {
            self.raw(48, &set.body())
        }

        /// Nests a VMTI Local Set (tag 74) built with [`st0903::Builder`].
        pub fn vmti(&mut self, set: &st0903::Builder) -> Result<&mut Self, Error> {
            self.raw(74, &set.body())
        }

        /// Image Horizon Pixel Pack (tag 81).
        pub fn image_horizon(&mut self, v: &packs::HorizonPixelPack) -> Result<&mut Self, Error> {
            self.raw(81, &v.encode()?)
        }

        /// One Control Command (tag 115); call again for each command.
        pub fn control_command(
            &mut self,
            v: &packs::ControlCommand<'_>,
        ) -> Result<&mut Self, Error> {
            self.raw(115, &v.encode()?)
        }

        /// Control Command Verification List (tag 116).
        pub fn verified_commands(&mut self, ids: &[u64]) -> Result<&mut Self, Error> {
            self.raw(116, &packs::encode_oid_list(ids)?)
        }

        /// Active Wavelength List (tag 121).
        pub fn active_wavelengths(&mut self, ids: &[u64]) -> Result<&mut Self, Error> {
            self.raw(121, &packs::encode_oid_list(ids)?)
        }

        /// Country Codes (tag 122).
        pub fn country_codes(&mut self, v: &packs::CountryCodes<'_>) -> Result<&mut Self, Error> {
            self.raw(122, &v.encode()?)
        }

        /// Sensor Frame Rate Pack (tag 127).
        pub fn frame_rate(&mut self, v: &packs::FrameRate) -> Result<&mut Self, Error> {
            self.raw(127, &v.encode()?)
        }

        /// Wavelengths List (tag 128).
        pub fn wavelengths(&mut self, list: &[packs::Wavelength<'_>]) -> Result<&mut Self, Error> {
            self.raw(128, &packs::encode_wavelengths(list)?)
        }

        /// Airbase Locations (tag 130).
        pub fn airbase_locations(
            &mut self,
            v: &packs::AirbaseLocations,
        ) -> Result<&mut Self, Error> {
            self.raw(130, &v.encode()?)
        }

        /// Payload List (tag 138).
        pub fn payloads(&mut self, list: &[packs::Payload<'_>]) -> Result<&mut Self, Error> {
            self.raw(138, &packs::encode_payloads(list)?)
        }

        /// Active Payloads (tag 139), from the ids that are on.
        pub fn active_payloads(&mut self, ids: &[u64]) -> Result<&mut Self, Error> {
            self.raw(139, &packs::encode_active_payloads(ids)?)
        }

        /// Waypoint List (tag 141).
        pub fn waypoints(&mut self, list: &[packs::Waypoint]) -> Result<&mut Self, Error> {
            self.raw(141, &packs::encode_waypoints(list)?)
        }

        /// View Domain (tag 142).
        pub fn view_domain(&mut self, v: &packs::ViewDomain) -> Result<&mut Self, Error> {
            self.raw(142, &v.encode()?)
        }

        /// Finishes the packet: key, length, items, version, checksum.
        pub fn finish(&self) -> Vec<u8> {
            let mut body = self.body.clone();
            if !self.has_version {
                enc::push_item(&mut body, 65, &[VERSION as u8]).expect("small item");
            }
            enc::finish_with_checksum(&KEY, &body).expect("packet under 4 GiB")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete_and_ordered() {
        assert_eq!(TAGS.len(), 143);
        for (i, t) in TAGS.iter().enumerate() {
            assert_eq!(t.tag as usize, i + 1);
            assert_eq!(Tag::from_u32(i as u32 + 1), Some(t.tag));
        }
        assert!(tag_info(0).is_none() && tag_info(144).is_none());
    }

    #[test]
    fn time_lengths() {
        assert_eq!(decode_tag(2, &[0]), Value::Invalid);
        assert_eq!(decode_tag(2, &[0, 0, 0, 0, 0, 0, 0, 7]), Value::Time(7));
        // Take-off Time in 7 bytes, as KWIVER writes it.
        assert_eq!(decode_tag(131, &[0, 0, 0, 0, 0, 1, 0]), Value::Time(256));
        assert_eq!(decode_tag(200, &[1]), Value::Bytes(&[1]));
    }

    #[test]
    fn linear_sentinels() {
        assert_eq!(
            decode_value(Tag::PlatformPitchAngle.info().kind, &[0x80, 0x00]),
            Value::OutOfRange
        );
        assert_eq!(
            decode_value(Tag::SensorLatitude.info().kind, &[0x80, 0, 0, 0]),
            Value::OutOfRange
        );
        assert_eq!(
            decode_value(Tag::SensorLatitude.info().kind, &[]),
            Value::Invalid
        );
        assert_eq!(
            decode_value(Tag::TargetWidth.info().kind, &[0, 0, 1, 0xC9]),
            Value::Float(457.0 * 10000.0 / 65535.0)
        );
        assert_eq!(
            decode_value(Tag::PlatformHeadingAngle.info().kind, &[0xFF, 0xFF]),
            Value::Float(360.0)
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn builder_round_trip() {
        let mut b = Builder::new(1_231_798_102_000_000);
        b.text(Tag::MissionId, "MISSION01").unwrap();
        b.float(Tag::PlatformPitchAngle, -0.4315251).unwrap();
        b.float(Tag::SensorLatitude, 60.1768229669783).unwrap();
        b.float(Tag::DensityAltitudeExtended, 23456.24).unwrap();
        b.float(Tag::PlatformCourseAngle, 29.157890122923).unwrap();
        b.int(Tag::OutsideAirTemperature, -12).unwrap();
        b.float(Tag::PlatformTrueAirspeed, 147.0).unwrap();
        let bytes = b.finish();
        let (p, n) = Packet::parse(&bytes).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(p.timestamp(), Some(1_231_798_102_000_000));
        assert_eq!(p.version(), Some(VERSION));
        assert_eq!(p.value(Tag::OutsideAirTemperature), Some(Value::Int(-12)));
        assert!((p.float(Tag::PlatformPitchAngle).unwrap() + 0.4315251).abs() < 40.0 / 65534.0);
        assert!((p.float(Tag::SensorLatitude).unwrap() - 60.1768229669783).abs() < 1e-7);
        assert!((p.float(Tag::DensityAltitudeExtended).unwrap() - 23456.24).abs() < 0.01);
        assert!(b.float(Tag::PlatformPitchAngle, 25.0).is_err());
        assert!(b.text(Tag::SensorLatitude, "x").is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn checksum_is_checked() {
        let mut bytes = Builder::new(5).finish();
        let n = bytes.len();
        bytes[n - 1] ^= 1;
        assert!(matches!(Packet::parse(&bytes), Err(Error::Checksum { .. })));
        assert!(Packet::parse_unverified(&bytes).is_ok());
    }
}
