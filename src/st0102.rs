//! MISB ST 0102 Security Local Set: the classification and releasability
//! markings of a motion imagery stream. In ST 0601 it is nested as tag 48.
//!
//! This module reads and writes the markings; it does not enforce them.

use crate::klv::{self, be_uint, Items};

/// ST 0102 tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum Tag {
    /// Security Classification (enumeration, see [`Classification`]).
    SecurityClassification = 1,
    /// Classifying Country and Releasing Instructions Country Coding Method.
    CountryCodingMethod = 2,
    /// Classifying Country (for example `//USA`).
    ClassifyingCountry = 3,
    /// SCI/SHI Information.
    SciShiInformation = 4,
    /// Caveats.
    Caveats = 5,
    /// Releasing Instructions.
    ReleasingInstructions = 6,
    /// Classified By.
    ClassifiedBy = 7,
    /// Derived From.
    DerivedFrom = 8,
    /// Classification Reason.
    ClassificationReason = 9,
    /// Declassification Date (`YYYYMMDD`).
    DeclassificationDate = 10,
    /// Classification and Marking System.
    ClassificationAndMarkingSystem = 11,
    /// Object Country Coding Method.
    ObjectCountryCodingMethod = 12,
    /// Object Country Codes (UTF-16 in current versions).
    ObjectCountryCodes = 13,
    /// Classification Comments.
    ClassificationComments = 14,
    /// Stream ID.
    StreamId = 19,
    /// Transport Stream ID.
    TransportStreamId = 20,
    /// Item Designator ID.
    ItemDesignatorId = 21,
    /// Version of ST 0102.
    Version = 22,
    /// Classifying Country Coding Method Version Date.
    CountryCodingMethodVersionDate = 23,
    /// Object Country Coding Method Version Date.
    ObjectCountryCodingMethodVersionDate = 24,
}

/// Security Classification values (tag 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Classification {
    /// `UNCLASSIFIED//`
    Unclassified,
    /// `RESTRICTED//`
    Restricted,
    /// `CONFIDENTIAL//`
    Confidential,
    /// `SECRET//`
    Secret,
    /// `TOP SECRET//`
    TopSecret,
    /// A value ST 0102 does not define.
    Other(u8),
}

impl Classification {
    /// From the tag 1 byte.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Classification::Unclassified,
            2 => Classification::Restricted,
            3 => Classification::Confidential,
            4 => Classification::Secret,
            5 => Classification::TopSecret,
            o => Classification::Other(o),
        }
    }

    /// The tag 1 byte.
    pub fn to_u8(self) -> u8 {
        match self {
            Classification::Unclassified => 1,
            Classification::Restricted => 2,
            Classification::Confidential => 3,
            Classification::Secret => 4,
            Classification::TopSecret => 5,
            Classification::Other(o) => o,
        }
    }

    /// The marking text, for example `"UNCLASSIFIED"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Classification::Unclassified => "UNCLASSIFIED",
            Classification::Restricted => "RESTRICTED",
            Classification::Confidential => "CONFIDENTIAL",
            Classification::Secret => "SECRET",
            Classification::TopSecret => "TOP SECRET",
            Classification::Other(_) => "UNKNOWN",
        }
    }
}

/// A Security Local Set body, borrowed from the packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecuritySet<'a> {
    body: &'a [u8],
}

impl<'a> SecuritySet<'a> {
    /// Wraps a local set body (the value of ST 0601 tag 48).
    pub fn new(body: &'a [u8]) -> Self {
        SecuritySet { body }
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

    /// Security Classification (tag 1).
    pub fn classification(&self) -> Option<Classification> {
        match self.raw(Tag::SecurityClassification)? {
            [v] => Some(Classification::from_u8(*v)),
            _ => None,
        }
    }

    /// A text tag (3 to 11, 14, 23, 24) as a string.
    pub fn text(&self, tag: Tag) -> Option<&'a str> {
        core::str::from_utf8(self.raw(tag)?).ok()
    }

    /// Classifying Country and Releasing Instructions Country Coding Method (tag 2).
    pub fn country_coding_method(&self) -> Option<u8> {
        self.raw(Tag::CountryCodingMethod)
            .and_then(be_uint)
            .map(|v| v as u8)
    }

    /// Object Country Codes (tag 13), decoded from UTF-16BE (current
    /// versions) or single-byte text (older senders).
    pub fn object_country_codes(&self) -> Option<CountryCodes<'a>> {
        self.raw(Tag::ObjectCountryCodes)
            .map(|raw| CountryCodes { raw })
    }

    /// ST 0102 version (tag 22).
    pub fn version(&self) -> Option<u64> {
        self.raw(Tag::Version).and_then(be_uint)
    }
}

/// Object Country Codes text, see [`SecuritySet::object_country_codes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryCodes<'a> {
    raw: &'a [u8],
}

impl<'a> CountryCodes<'a> {
    /// The raw bytes.
    pub fn raw(&self) -> &'a [u8] {
        self.raw
    }

    fn is_utf16(&self) -> bool {
        self.raw.len() % 2 == 0
            && !self.raw.is_empty()
            && self.raw.iter().step_by(2).all(|&b| b == 0)
    }

    /// The characters.
    pub fn chars(&self) -> impl Iterator<Item = char> + 'a {
        let utf16 = self.is_utf16();
        let raw = self.raw;
        let units = (0..if utf16 { raw.len() / 2 } else { raw.len() }).map(move |i| {
            if utf16 {
                u16::from_be_bytes([raw[2 * i], raw[2 * i + 1]])
            } else {
                raw[i] as u16
            }
        });
        core::char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    /// Compares with `s`.
    pub fn eq_str(&self, s: &str) -> bool {
        self.chars().eq(s.chars())
    }
}

#[cfg(feature = "alloc")]
pub use builder::Builder;

#[cfg(feature = "alloc")]
mod builder {
    use super::*;
    use crate::klv::enc;
    use crate::Error;
    use alloc::vec::Vec;

    /// Encodes a Security Local Set, to nest with [`crate::st0601::Builder::security`].
    #[derive(Clone, Debug)]
    pub struct Builder {
        body: Vec<u8>,
    }

    impl Builder {
        /// Starts a set with the classification, coding method 7
        /// (ISO-3166 three-letter, the common choice) and the classifying country.
        pub fn new(classification: Classification, classifying_country: &str) -> Self {
            let mut body = Vec::new();
            enc::push_item(&mut body, 1, &[classification.to_u8()]).expect("small item");
            enc::push_item(&mut body, 2, &[7]).expect("small item");
            enc::push_item(&mut body, 3, classifying_country.as_bytes()).expect("small item");
            Builder { body }
        }

        /// Adds a text tag.
        pub fn text(&mut self, tag: Tag, value: &str) -> Result<&mut Self, Error> {
            if tag == Tag::ObjectCountryCodes {
                return self.object_country_codes(value);
            }
            enc::push_item(&mut self.body, tag as u32, value.as_bytes())?;
            Ok(self)
        }

        /// Adds Object Country Coding Method 7 and the Object Country Codes (UTF-16BE).
        pub fn object_country_codes(&mut self, value: &str) -> Result<&mut Self, Error> {
            let units: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
            enc::push_item(&mut self.body, 12, &[7])?;
            enc::push_item(&mut self.body, 13, &units)?;
            Ok(self)
        }

        /// Adds a raw item.
        pub fn raw(&mut self, tag: u32, value: &[u8]) -> Result<&mut Self, Error> {
            enc::push_item(&mut self.body, tag, value)?;
            Ok(self)
        }

        /// The encoded body, with the version item (22) appended.
        pub fn body(&self) -> Vec<u8> {
            let mut b = self.body.clone();
            enc::push_item(&mut b, 22, &12u16.to_be_bytes()).expect("small item");
            b
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sample_set() {
        // From the MISB "Dynamic and Constant" sample packet.
        let body = [
            0x01, 0x01, 0x01, 0x02, 0x01, 0x07, 0x03, 0x05, b'/', b'/', b'U', b'S', b'A', 0x0C,
            0x01, 0x07, 0x0D, 0x06, 0x00, 0x55, 0x00, 0x53, 0x00, 0x41, 0x16, 0x02, 0x00, 0x0A,
        ];
        let s = SecuritySet::new(&body);
        assert_eq!(s.classification(), Some(Classification::Unclassified));
        assert_eq!(s.text(Tag::ClassifyingCountry), Some("//USA"));
        assert!(s.object_country_codes().unwrap().eq_str("USA"));
        assert_eq!(s.version(), Some(10));
        assert_eq!(s.country_coding_method(), Some(7));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn builder_round_trip() {
        let mut b = Builder::new(Classification::Unclassified, "//USA");
        b.object_country_codes("USA;CAN").unwrap();
        let body = b.body();
        let s = SecuritySet::new(&body);
        assert_eq!(s.classification(), Some(Classification::Unclassified));
        assert!(s.object_country_codes().unwrap().eq_str("USA;CAN"));
        assert_eq!(s.version(), Some(12));
    }
}
