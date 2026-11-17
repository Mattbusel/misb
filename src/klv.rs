//! KLV building blocks (SMPTE ST 336, MISB ST 0107): universal keys, BER
//! lengths, BER-OID tags, local set items, the 16-bit packet checksum and a
//! splitter that finds packets in a raw byte stream.

use crate::Error;

/// The first four bytes of every SMPTE universal key (`06 0E 2B 34`).
pub const UL_PREFIX: [u8; 4] = [0x06, 0x0E, 0x2B, 0x34];

/// Longest BER long-form length accepted: `0x84` plus four bytes.
const MAX_BER_LONG: usize = 4;

/// Reads a BER length at the start of `buf`.
///
/// Returns `(length, bytes used)`. Short form (`0x00..=0x7F`) and long form
/// (`0x81..=0x84` followed by that many bytes) are accepted; `0x80`
/// (indefinite) and anything longer than four bytes is [`Error::BadLength`].
pub fn read_ber_length(buf: &[u8]) -> Result<(usize, usize), Error> {
    let first = *buf.first().ok_or(Error::Truncated)?;
    if first < 0x80 {
        return Ok((first as usize, 1));
    }
    let n = (first & 0x7F) as usize;
    if n == 0 || n > MAX_BER_LONG {
        return Err(Error::BadLength);
    }
    let bytes = buf.get(1..1 + n).ok_or(Error::Truncated)?;
    let mut len: u64 = 0;
    for &b in bytes {
        len = (len << 8) | b as u64;
    }
    let len = usize::try_from(len).map_err(|_| Error::BadLength)?;
    Ok((len, 1 + n))
}

/// Writes the shortest BER length for `len` into `out`; returns the bytes used.
pub fn write_ber_length(len: usize, out: &mut [u8]) -> Result<usize, Error> {
    if len < 0x80 {
        *out.first_mut().ok_or(Error::Truncated)? = len as u8;
        return Ok(1);
    }
    let len = len as u64;
    let n = ((64 - len.leading_zeros() + 7) / 8) as usize;
    if n > MAX_BER_LONG {
        return Err(Error::BadLength);
    }
    let out = out.get_mut(..1 + n).ok_or(Error::Truncated)?;
    out[0] = 0x80 | n as u8;
    for i in 0..n {
        out[1 + i] = (len >> (8 * (n - 1 - i))) as u8;
    }
    Ok(1 + n)
}

/// Reads a BER-OID encoded integer (7 bits per byte, high bit = more).
///
/// Used for local set tags (ST 0601 tags above 127 take two bytes) and
/// VMTI target ids. Up to 9 bytes (63 bits) are accepted.
pub fn read_ber_oid(buf: &[u8]) -> Result<(u64, usize), Error> {
    let mut v: u64 = 0;
    for (i, &b) in buf.iter().enumerate() {
        if i == 9 {
            return Err(Error::BadLength);
        }
        v = (v << 7) | (b & 0x7F) as u64;
        if b & 0x80 == 0 {
            return Ok((v, i + 1));
        }
    }
    Err(Error::Truncated)
}

/// Writes `v` as BER-OID into `out`; returns the bytes used.
pub fn write_ber_oid(v: u64, out: &mut [u8]) -> Result<usize, Error> {
    if v >= 1 << 63 {
        return Err(Error::BadLength);
    }
    let bits = 64 - v.leading_zeros();
    let n = core::cmp::max(1, ((bits + 6) / 7) as usize);
    let out = out.get_mut(..n).ok_or(Error::Truncated)?;
    for (i, o) in out.iter_mut().enumerate() {
        let shift = 7 * (n - 1 - i);
        *o = ((v >> shift) & 0x7F) as u8 | if i + 1 < n { 0x80 } else { 0 };
    }
    Ok(n)
}

/// One item of a local set: a tag and its raw value bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawItem<'a> {
    /// Local set tag (BER-OID decoded).
    pub tag: u32,
    /// Value bytes, borrowed from the packet.
    pub value: &'a [u8],
}

/// Iterator over the items of a local set body.
///
/// A malformed item yields one `Err` and then the iterator ends, since
/// nothing after it can be located reliably.
#[derive(Clone, Debug)]
pub struct Items<'a> {
    rest: &'a [u8],
    failed: bool,
}

impl<'a> Items<'a> {
    /// Iterates the items in `body` (a local set value, without key and length).
    pub fn new(body: &'a [u8]) -> Self {
        Items {
            rest: body,
            failed: false,
        }
    }

    fn step(&mut self) -> Result<RawItem<'a>, Error> {
        let (tag, n) = read_ber_oid(self.rest)?;
        let tag = u32::try_from(tag).map_err(|_| Error::BadLength)?;
        let (len, m) = read_ber_length(&self.rest[n..])?;
        let start = n + m;
        let end = start.checked_add(len).ok_or(Error::BadLength)?;
        let value = self.rest.get(start..end).ok_or(Error::Truncated)?;
        self.rest = &self.rest[end..];
        Ok(RawItem { tag, value })
    }
}

impl<'a> Iterator for Items<'a> {
    type Item = Result<RawItem<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() || self.failed {
            return None;
        }
        let r = self.step();
        if r.is_err() {
            self.failed = true;
        }
        Some(r)
    }
}

/// Returns the first item with `tag` in a local set body, ignoring a
/// malformed tail.
pub fn find(body: &[u8], tag: u32) -> Option<&[u8]> {
    Items::new(body)
        .map_while(Result::ok)
        .find(|i| i.tag == tag)
        .map(|i| i.value)
}

/// The 16-bit running sum used by ST 0601 and ST 0903 (tag 1).
///
/// Covers every byte from the start of the 16-byte key through the length
/// byte of the checksum item, which means the whole packet except its last
/// two bytes.
pub fn checksum(data: &[u8]) -> u16 {
    let mut bcc: u16 = 0;
    for (i, &b) in data.iter().enumerate() {
        bcc = bcc.wrapping_add((b as u16) << (8 * ((i + 1) % 2)));
    }
    bcc
}

/// Reads the unsigned big-endian integer in `b` (1 to 8 bytes).
pub fn be_uint(b: &[u8]) -> Option<u64> {
    if b.is_empty() || b.len() > 8 {
        return None;
    }
    Some(b.iter().fold(0u64, |v, &x| (v << 8) | x as u64))
}

/// Reads the signed (two's complement) big-endian integer in `b` (1 to 8 bytes).
pub fn be_int(b: &[u8]) -> Option<i64> {
    let u = be_uint(b)?;
    let bits = 8 * b.len() as u32;
    Some(if bits == 64 {
        u as i64
    } else {
        ((u << (64 - bits)) as i64) >> (64 - bits)
    })
}

/// Compares two universal keys, ignoring byte 8 (the registry version
/// byte, index 7), which SMPTE ST 336 says not to use for matching and
/// which encoders fill differently.
pub fn same_key(a: &[u8], b: &[u8]) -> bool {
    a.len() == 16
        && b.len() == 16
        && a.iter()
            .zip(b)
            .enumerate()
            .all(|(i, (x, y))| i == 7 || x == y)
}

/// A KLV unit found by [`split`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit<'a> {
    /// The universal key.
    pub key: &'a [u8; 16],
    /// The value bytes.
    pub value: &'a [u8],
    /// Key, length and value, as needed for checksums and for
    /// [`crate::st0601::Packet::parse`].
    pub raw: &'a [u8],
}

/// Splits a byte stream (for example the KLV data stream extracted from an
/// MPEG-TS file with `ffmpeg -map 0:d -c copy -f data`) into KLV units.
///
/// Bytes that are not the start of a universal key are skipped, so the
/// splitter resynchronises after garbage or a truncated unit.
pub fn split(stream: &[u8]) -> Split<'_> {
    Split { rest: stream }
}

/// Iterator returned by [`split`].
#[derive(Clone, Debug)]
pub struct Split<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for Split<'a> {
    type Item = Unit<'a>;

    fn next(&mut self) -> Option<Unit<'a>> {
        loop {
            let pos = self.rest.windows(4).position(|w| w == UL_PREFIX)?;
            let s = &self.rest[pos..];
            if s.len() < 17 {
                self.rest = &[];
                return None;
            }
            match read_ber_length(&s[16..]) {
                Ok((len, n)) => {
                    let end = 16usize.saturating_add(n).saturating_add(len);
                    if end > s.len() {
                        // A truncated unit, or the prefix bytes inside some value.
                        self.rest = &s[1..];
                        continue;
                    }
                    let key: &[u8; 16] = s[..16].try_into().ok()?;
                    let unit = Unit {
                        key,
                        value: &s[16 + n..end],
                        raw: &s[..end],
                    };
                    self.rest = &s[end..];
                    return Some(unit);
                }
                Err(_) => self.rest = &s[1..],
            }
        }
    }
}

/// Checks the 16-byte key, the BER length and the trailing checksum item of
/// a packet that starts at `buf[0]`. Returns `(value, bytes used)`.
pub(crate) fn parse_checked<'a>(
    buf: &'a [u8],
    key: &[u8; 16],
    verify: bool,
) -> Result<(&'a [u8], usize), Error> {
    let k = buf.get(..16).ok_or(Error::Truncated)?;
    if !same_key(k, key) {
        return Err(Error::WrongKey);
    }
    let (len, n) = read_ber_length(&buf[16..])?;
    let start = 16 + n;
    let end = start.checked_add(len).ok_or(Error::BadLength)?;
    let value = buf.get(start..end).ok_or(Error::Truncated)?;
    if verify {
        if value.len() < 4 || value[value.len() - 4..value.len() - 2] != [0x01, 0x02] {
            return Err(Error::MissingChecksum);
        }
        let found = u16::from_be_bytes([buf[end - 2], buf[end - 1]]);
        let computed = checksum(&buf[..end - 2]);
        if found != computed {
            return Err(Error::Checksum { found, computed });
        }
    }
    Ok((value, end))
}

#[cfg(feature = "alloc")]
pub(crate) mod enc {
    use super::*;
    use alloc::vec::Vec;

    pub(crate) fn push_ber_length(out: &mut Vec<u8>, len: usize) -> Result<(), Error> {
        let mut b = [0u8; 5];
        let n = write_ber_length(len, &mut b)?;
        out.extend_from_slice(&b[..n]);
        Ok(())
    }

    pub(crate) fn push_item(out: &mut Vec<u8>, tag: u32, value: &[u8]) -> Result<(), Error> {
        let mut b = [0u8; 10];
        let n = write_ber_oid(tag as u64, &mut b)?;
        out.extend_from_slice(&b[..n]);
        push_ber_length(out, value.len())?;
        out.extend_from_slice(value);
        Ok(())
    }

    /// Smallest big-endian representation of `v` (at least one byte).
    pub(crate) fn min_uint(v: u64) -> Vec<u8> {
        let n = core::cmp::max(1, (64 - v.leading_zeros() as usize + 7) / 8);
        v.to_be_bytes()[8 - n..].to_vec()
    }

    /// Wraps `body` as key + BER length + body + checksum item.
    pub(crate) fn finish_with_checksum(key: &[u8; 16], body: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(body.len() + 24);
        out.extend_from_slice(key);
        push_ber_length(&mut out, body.len() + 4)?;
        out.extend_from_slice(body);
        out.extend_from_slice(&[0x01, 0x02]);
        let c = checksum(&out);
        out.extend_from_slice(&c.to_be_bytes());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ber_lengths() {
        assert_eq!(read_ber_length(&[0x7F]), Ok((127, 1)));
        assert_eq!(read_ber_length(&[0x81, 0xC9]), Ok((201, 2)));
        assert_eq!(read_ber_length(&[0x82, 0x01, 0x00]), Ok((256, 3)));
        assert_eq!(read_ber_length(&[0x80]), Err(Error::BadLength));
        assert_eq!(
            read_ber_length(&[0x85, 0, 0, 0, 0, 1]),
            Err(Error::BadLength)
        );
        assert_eq!(read_ber_length(&[0x82, 0x01]), Err(Error::Truncated));
        let mut b = [0u8; 5];
        for len in [0usize, 1, 127, 128, 255, 256, 65535, 65536, 1 << 24] {
            let n = write_ber_length(len, &mut b).unwrap();
            assert_eq!(read_ber_length(&b[..n]), Ok((len, n)));
        }
    }

    #[test]
    fn ber_oids() {
        assert_eq!(read_ber_oid(&[0x01]), Ok((1, 1)));
        assert_eq!(read_ber_oid(&[0x81, 0x00]), Ok((128, 2)));
        assert_eq!(read_ber_oid(&[0x81, 0x0F]), Ok((143, 2)));
        assert_eq!(read_ber_oid(&[0x81]), Err(Error::Truncated));
        assert_eq!(read_ber_oid(&[0xFF; 12]), Err(Error::BadLength));
        let mut b = [0u8; 10];
        for v in [
            0u64,
            1,
            127,
            128,
            143,
            16383,
            16384,
            u32::MAX as u64,
            (1 << 63) - 1,
        ] {
            let n = write_ber_oid(v, &mut b).unwrap();
            assert_eq!(read_ber_oid(&b[..n]), Ok((v, n)));
        }
    }

    #[test]
    fn key_version_byte_is_ignored() {
        let mut k = crate::st0601::KEY;
        assert!(same_key(&k, &crate::st0601::KEY));
        k[7] = 0x0E;
        assert!(same_key(&k, &crate::st0601::KEY));
        k[8] = 0;
        assert!(!same_key(&k, &crate::st0601::KEY));
        assert!(!same_key(&k[..15], &crate::st0601::KEY));
    }

    #[test]
    fn signed_ints() {
        assert_eq!(be_int(&[0xFF]), Some(-1));
        assert_eq!(be_int(&[0x80, 0x00]), Some(-32768));
        assert_eq!(be_int(&[0x7F, 0xFF, 0xFF, 0xFF]), Some(i32::MAX as i64));
        assert_eq!(be_int(&[]), None);
    }

    #[test]
    fn items_stop_on_garbage() {
        let body = [0x02, 0x01, 0xAA, 0x03, 0x05, 0x01];
        let mut it = Items::new(&body);
        assert_eq!(
            it.next(),
            Some(Ok(RawItem {
                tag: 2,
                value: &[0xAA]
            }))
        );
        assert_eq!(it.next(), Some(Err(Error::Truncated)));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn split_resyncs() {
        let mut s = alloc_free_stream();
        // garbage, a unit with a lying length, then a good unit
        let good = [
            0x06, 0x0E, 0x2B, 0x34, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 2, 0xAB, 0xCD,
        ];
        s[..3].copy_from_slice(&[0xFF, 0x06, 0x0E]);
        s[3..22].copy_from_slice(&good);
        let units: [Option<Unit<'_>>; 2] = {
            let mut it = split(&s[..22]);
            [it.next(), it.next()]
        };
        assert_eq!(units[0].unwrap().value, &[0xAB, 0xCD]);
        assert!(units[1].is_none());
    }

    fn alloc_free_stream() -> [u8; 32] {
        [0u8; 32]
    }
}
