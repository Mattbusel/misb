//! Python bindings: `misb.parse`, `misb.decode_item`, `misb.Builder` and
//! the KLV helpers.

use misb::st0601::{self, Kind, Tag, Value};
use misb::{klv, st0102, st0903, Error, GeoPoint};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

fn bad(e: Error) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Text => "text",
        Kind::Time => "time",
        Kind::Uint => "uint",
        Kind::Int => "int",
        Kind::Linear { .. } => "linear",
        Kind::Imapb { .. } => "imapb",
        Kind::Security => "security",
        Kind::Vmti => "vmti",
        Kind::Bytes => "bytes",
    }
}

fn known(tag: u32) -> PyResult<Tag> {
    st0601::tag_info(tag)
        .map(|i| i.tag)
        .ok_or_else(|| PyValueError::new_err(format!("tag {tag} is not defined in ST 0601.19")))
}

fn point<'py>(py: Python<'py>, p: GeoPoint) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("lat", p.lat)?;
    d.set_item("lon", p.lon)?;
    d.set_item("height", p.height)?;
    Ok(d)
}

fn set_opt<'py, T: IntoPyObject<'py>>(
    d: &Bound<'py, PyDict>,
    k: &str,
    v: Option<T>,
) -> PyResult<()> {
    if let Some(v) = v {
        d.set_item(k, v)?;
    }
    Ok(())
}

fn security_dict<'py>(py: Python<'py>, s: &st0102::SecuritySet) -> PyResult<Bound<'py, PyDict>> {
    use st0102::Tag as T;
    let d = PyDict::new(py);
    if let Some(c) = s.classification() {
        d.set_item("classification", c.as_str())?;
        d.set_item("classification_code", c.to_u8())?;
    }
    set_opt(&d, "country_coding_method", s.country_coding_method())?;
    for (key, tag) in [
        ("classifying_country", T::ClassifyingCountry),
        ("sci_shi_information", T::SciShiInformation),
        ("caveats", T::Caveats),
        ("releasing_instructions", T::ReleasingInstructions),
        ("classified_by", T::ClassifiedBy),
        ("derived_from", T::DerivedFrom),
        ("classification_reason", T::ClassificationReason),
        ("declassification_date", T::DeclassificationDate),
        (
            "classification_and_marking_system",
            T::ClassificationAndMarkingSystem,
        ),
        ("classification_comments", T::ClassificationComments),
    ] {
        set_opt(&d, key, s.text(tag))?;
    }
    set_opt(
        &d,
        "object_country_coding_method",
        s.raw(T::ObjectCountryCodingMethod).and_then(klv::be_uint),
    )?;
    set_opt(
        &d,
        "object_country_codes",
        s.object_country_codes()
            .map(|c| c.chars().collect::<String>()),
    )?;
    set_opt(&d, "version", s.version())?;
    Ok(d)
}

fn vmti_dict<'py>(py: Python<'py>, v: &st0903::VmtiSet) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    set_opt(&d, "timestamp", v.timestamp())?;
    set_opt(&d, "system_name", v.system_name())?;
    set_opt(&d, "source_sensor", v.source_sensor())?;
    set_opt(&d, "version", v.version())?;
    set_opt(&d, "total_targets_detected", v.total_targets_detected())?;
    set_opt(&d, "targets_reported", v.targets_reported())?;
    set_opt(&d, "frame_number", v.frame_number())?;
    set_opt(&d, "frame_width", v.frame_width())?;
    set_opt(&d, "frame_height", v.frame_height())?;
    set_opt(&d, "horizontal_fov", v.horizontal_fov())?;
    set_opt(&d, "vertical_fov", v.vertical_fov())?;
    let targets = PyList::empty(py);
    for t in v.targets() {
        let t = match t {
            Ok(t) => t,
            Err(e) => {
                d.set_item("targets_error", e.to_string())?;
                break;
            }
        };
        let td = PyDict::new(py);
        td.set_item("id", t.id)?;
        set_opt(&td, "centroid_pixel", t.centroid_pixel())?;
        if let Some(w) = v.frame_width() {
            set_opt(&td, "centroid_row_col", t.centroid_row_col(w))?;
        }
        set_opt(&td, "bounding_box", t.bounding_box())?;
        set_opt(&td, "priority", t.priority())?;
        set_opt(&td, "confidence", t.confidence())?;
        set_opt(&td, "history", t.history())?;
        set_opt(&td, "color", t.color())?;
        set_opt(&td, "location_offset", t.location_offset())?;
        set_opt(&td, "height", t.height())?;
        if let Some(p) = t.location() {
            td.set_item("location", point(py, p)?)?;
        }
        set_opt(&td, "algorithm_id", t.algorithm_id())?;
        targets.append(td)?;
    }
    d.set_item("targets", targets)?;
    Ok(d)
}

/// (status, value) for a decoded value.
fn value_obj<'py>(py: Python<'py>, v: Value) -> PyResult<(&'static str, Bound<'py, PyAny>)> {
    Ok(match v {
        Value::Float(f) => ("ok", f.into_pyobject(py)?.into_any()),
        Value::Uint(u) | Value::Time(u) => ("ok", u.into_pyobject(py)?.into_any()),
        Value::Int(i) => ("ok", i.into_pyobject(py)?.into_any()),
        Value::Text(s) => ("ok", s.into_pyobject(py)?.into_any()),
        Value::Security(s) => ("ok", security_dict(py, &s)?.into_any()),
        Value::Vmti(s) => ("ok", vmti_dict(py, &s)?.into_any()),
        Value::Bytes(b) => ("raw", PyBytes::new(py, b).into_any()),
        Value::OutOfRange => ("out_of_range", py.None().into_bound(py)),
        Value::Invalid => ("invalid", py.None().into_bound(py)),
    })
}

fn item_dict<'py>(py: Python<'py>, tag: u32, raw: &[u8]) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("tag", tag)?;
    match st0601::tag_info(tag) {
        Some(info) => {
            d.set_item("name", info.name)?;
            d.set_item("units", info.units)?;
            d.set_item("kind", kind_name(info.kind))?;
            let (status, value) = value_obj(py, st0601::decode_tag(tag, raw))?;
            d.set_item("status", status)?;
            d.set_item("value", value)?;
        }
        None => {
            d.set_item("name", format!("Unknown Tag {tag}"))?;
            d.set_item("units", "")?;
            d.set_item("kind", "bytes")?;
            d.set_item("status", "raw")?;
            d.set_item("value", PyBytes::new(py, raw))?;
        }
    }
    d.set_item("raw", PyBytes::new(py, raw))?;
    Ok(d)
}

/// Decodes one ST 0601 item value.
///
/// Returns a dict: ``tag``, ``name``, ``units``, ``kind``, ``raw`` (bytes),
/// ``status`` and ``value``. ``status`` is ``"ok"`` (``value`` is a float,
/// int, str, or a dict for the ST 0102 security set and the ST 0903 VMTI
/// set), ``"out_of_range"`` (the sender's out-of-range marker, ``value`` is
/// None), ``"invalid"`` (wrong length or not text, ``value`` is None) or
/// ``"raw"`` (packs, other nested sets and unknown tags, ``value`` is bytes).
#[pyfunction]
fn decode_item<'py>(py: Python<'py>, tag: u32, raw: &[u8]) -> PyResult<Bound<'py, PyDict>> {
    item_dict(py, tag, raw)
}

/// Parses a UAS Datalink Local Set packet at the start of ``data``.
///
/// Checks the 16-byte key (ignoring the registry version byte), the BER
/// length and, unless ``verify`` is False, the checksum. Returns
/// ``(items, used)``: every item in packet order as a ``decode_item`` dict,
/// and the number of bytes the packet took. Raises ValueError otherwise.
#[pyfunction]
#[pyo3(signature = (data, verify = true))]
fn parse<'py>(py: Python<'py>, data: &[u8], verify: bool) -> PyResult<(Bound<'py, PyList>, usize)> {
    let (packet, used) = if verify {
        st0601::Packet::parse(data)
    } else {
        st0601::Packet::parse_unverified(data)
    }
    .map_err(bad)?;
    let items = PyList::empty(py);
    for item in packet.items() {
        let item = item.map_err(bad)?;
        items.append(item_dict(py, item.tag, item.raw)?)?;
    }
    Ok((items, used))
}

/// Name, units and kind of an ST 0601 tag, or None for an undefined tag.
#[pyfunction]
fn tag_info<'py>(py: Python<'py>, tag: u32) -> PyResult<Option<Bound<'py, PyDict>>> {
    let Some(info) = st0601::tag_info(tag) else {
        return Ok(None);
    };
    let d = PyDict::new(py);
    d.set_item("tag", tag)?;
    d.set_item("name", info.name)?;
    d.set_item("units", info.units)?;
    d.set_item("kind", kind_name(info.kind))?;
    match info.kind {
        Kind::Linear {
            len,
            signed,
            min,
            max,
        } => {
            d.set_item("length", len)?;
            d.set_item("signed", signed)?;
            d.set_item("min", min)?;
            d.set_item("max", max)?;
        }
        Kind::Imapb { min, max, len } => {
            d.set_item("length", len)?;
            d.set_item("min", min)?;
            d.set_item("max", max)?;
        }
        _ => {}
    }
    Ok(Some(d))
}

/// The 16-bit ST 0601 checksum of ``data`` (key through the checksum
/// item's length byte).
#[pyfunction]
fn checksum(data: &[u8]) -> u16 {
    klv::checksum(data)
}

/// Splits a byte stream (for example ``ffmpeg -map 0:d -c copy -f data``
/// output) into KLV units. Returns a list of ``(key, value, raw)`` bytes
/// tuples; ``raw`` is the whole unit and can go straight to ``parse``.
/// Garbage between units is skipped.
#[pyfunction]
fn split<'py>(py: Python<'py>, stream: &[u8]) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    for u in klv::split(stream) {
        out.append((
            PyBytes::new(py, u.key),
            PyBytes::new(py, u.value),
            PyBytes::new(py, u.raw),
        ))?;
    }
    Ok(out)
}

/// Encodes a UAS Datalink Local Set packet.
///
/// ``Builder(timestamp_us)`` starts the packet with the Precision Time
/// Stamp. Items are written in the order they are added; ``finish()``
/// appends the version (tag 65, unless set) and the checksum and returns
/// the packet bytes. Values outside a tag's range raise ValueError instead
/// of being clamped.
#[pyclass(module = "misb")]
struct Builder {
    inner: st0601::Builder,
}

#[pymethods]
impl Builder {
    #[new]
    fn new(timestamp_us: u64) -> Self {
        Builder {
            inner: st0601::Builder::new(timestamp_us),
        }
    }

    /// A number in the tag's units (linear, IMAPB and integer tags).
    /// ``length`` overrides the byte length of an IMAPB tag.
    #[pyo3(signature = (tag, value, length = None))]
    fn float(
        mut slf: PyRefMut<'_, Self>,
        tag: u32,
        value: f64,
        length: Option<usize>,
    ) -> PyResult<PyRefMut<'_, Self>> {
        let t = known(tag)?;
        match length {
            Some(n) => slf.inner.float_len(t, value, n),
            None => slf.inner.float(t, value),
        }
        .map_err(bad)?;
        Ok(slf)
    }

    /// An unsigned integer tag (counts, enumerations, flags).
    fn uint(mut slf: PyRefMut<'_, Self>, tag: u32, value: u64) -> PyResult<PyRefMut<'_, Self>> {
        let t = known(tag)?;
        slf.inner.uint(t, value).map_err(bad)?;
        Ok(slf)
    }

    /// A signed integer tag.
    fn int(mut slf: PyRefMut<'_, Self>, tag: u32, value: i64) -> PyResult<PyRefMut<'_, Self>> {
        let t = known(tag)?;
        slf.inner.int(t, value).map_err(bad)?;
        Ok(slf)
    }

    /// A text tag.
    fn text<'a>(
        mut slf: PyRefMut<'a, Self>,
        tag: u32,
        value: &str,
    ) -> PyResult<PyRefMut<'a, Self>> {
        let t = known(tag)?;
        slf.inner.text(t, value).map_err(bad)?;
        Ok(slf)
    }

    /// A time tag, microseconds since 1970 UTC.
    fn time(mut slf: PyRefMut<'_, Self>, tag: u32, us: u64) -> PyResult<PyRefMut<'_, Self>> {
        let t = known(tag)?;
        slf.inner.time(t, us).map_err(bad)?;
        Ok(slf)
    }

    /// Raw bytes for any tag except the checksum (packs, nested sets).
    fn raw<'a>(
        mut slf: PyRefMut<'a, Self>,
        tag: u32,
        value: &[u8],
    ) -> PyResult<PyRefMut<'a, Self>> {
        slf.inner.raw(tag, value).map_err(bad)?;
        Ok(slf)
    }

    /// The finished packet: key, length, items, version, checksum.
    fn finish<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.finish())
    }
}

#[pymodule]
fn _misb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("UAS_LOCAL_SET_KEY", PyBytes::new(m.py(), &st0601::KEY))?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(decode_item, m)?)?;
    m.add_function(wrap_pyfunction!(tag_info, m)?)?;
    m.add_function(wrap_pyfunction!(checksum, m)?)?;
    m.add_function(wrap_pyfunction!(split, m)?)?;
    m.add_class::<Builder>()?;
    Ok(())
}
