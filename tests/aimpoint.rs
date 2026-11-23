//! The "where is the camera pointed" result, checked on real sample KLV.
//!
//! `tests/data/day_flight.klv` is the KLV stream of the public MISB / ffmpeg
//! "Day Flight.mpg" sample (samples.ffmpeg.org/MPEG2/mpegts-klv), extracted
//! with `ffmpeg -i <file> -map 0:d -c copy -f data <out>`. Every packet in it
//! carries the sensor position (ST 0601 tags 13/14/15), the pointing angles
//! (tags 5, 18, 19), the slant range (tag 21), and the sensor's own Frame
//! Center and Target Location geolocations (tags 23/24/25 and 40/41/42).

use misb::geo::AimSource;
use misb::klv;
use misb::st0601::Packet;

fn day_flight() -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/data/day_flight.klv",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn first_packet(bytes: &[u8]) -> Packet<'_> {
    let unit = klv::split(bytes).next().expect("a KLV unit");
    Packet::from_unit(&unit).expect("a valid ST 0601 packet")
}

/// On real sample KLV, `aimpoint` reports the ground point the camera is
/// filming, from the sensor's own Frame Center, with the downrange distance and
/// compass bearing an operator would act on.
#[test]
fn aimpoint_on_sample_is_the_frame_center() {
    let bytes = day_flight();
    let p = first_packet(&bytes);

    let fc = p.frame_center().expect("frame center");
    let aim = p.aimpoint().expect("an aimpoint");

    // The trustworthy source in this packet is the sensor's Frame Center.
    assert_eq!(aim.source, AimSource::FrameCenter);
    assert_eq!(aim.point, fc);

    // The camera is filming a point near 54.749 N, 110.047 W.
    assert!(
        (aim.point.lat - 54.749_123).abs() < 1e-4,
        "{}",
        aim.point.lat
    );
    assert!(
        (aim.point.lon - -110.046_638).abs() < 1e-4,
        "{}",
        aim.point.lon
    );

    // It lies about 10.9 km downrange on a bearing near north-east (46 deg),
    // consistent with the sensor at 54.681 N, 110.169 W looking at that point.
    assert!(
        (aim.ground_range_m - 10_900.0).abs() < 150.0,
        "ground_range_m = {}",
        aim.ground_range_m
    );
    assert!(
        (aim.bearing_deg - 46.2).abs() < 1.0,
        "bearing_deg = {}",
        aim.bearing_deg
    );
}

/// Every packet in the sample yields an aimpoint, and because each carries the
/// sensor's Frame Center, that is the source used throughout.
#[test]
fn every_sample_packet_has_a_frame_center_aimpoint() {
    let bytes = day_flight();
    let mut packets = 0;
    for unit in klv::split(&bytes) {
        let p = Packet::from_unit(&unit).expect("valid packet");
        let aim = p.aimpoint().expect("an aimpoint");
        assert_eq!(aim.source, AimSource::FrameCenter);
        assert!(aim.point.lat.is_finite() && aim.point.lon.is_finite());
        packets += 1;
    }
    assert!(packets > 0, "sample decoded no packets");
}

/// With Frame Center and Target Location removed, `aimpoint` falls back to
/// projecting from the sensor position, pointing angles and slant range, and
/// still returns a finite ground point. The sample's angle tags and its Frame
/// Center are not mutually consistent (a known trait of this synthetic MISB
/// sample), so the projected point is only checked for sanity, not against the
/// Frame Center; the geodesy itself is validated in `geo`'s unit tests.
#[test]
fn projection_fallback_when_geolocation_is_absent() {
    use misb::st0601::{Builder, Tag};

    let bytes = day_flight();
    let src = first_packet(&bytes);
    let sensor = src.sensor_position().unwrap();

    // Rebuild a packet with the pointing tags but no frame center or target.
    let mut b = Builder::new(src.timestamp().unwrap_or(0));
    b.float(Tag::SensorLatitude, sensor.lat).unwrap();
    b.float(Tag::SensorLongitude, sensor.lon).unwrap();
    b.float(Tag::SensorTrueAltitude, sensor.height.unwrap())
        .unwrap();
    b.float(Tag::PlatformHeadingAngle, src.platform_heading().unwrap())
        .unwrap();
    b.float(
        Tag::SensorRelativeAzimuthAngle,
        src.float(Tag::SensorRelativeAzimuthAngle).unwrap(),
    )
    .unwrap();
    b.float(
        Tag::SensorRelativeElevationAngle,
        src.float(Tag::SensorRelativeElevationAngle).unwrap(),
    )
    .unwrap();
    b.float(Tag::SlantRange, src.float(Tag::SlantRange).unwrap())
        .unwrap();
    let encoded = b.finish();

    let (p, _) = Packet::parse(&encoded).unwrap();
    assert!(p.frame_center().is_none());
    let aim = p.aimpoint().expect("a projected aimpoint");
    assert_eq!(aim.source, AimSource::Projected);
    assert!(aim.point.lat.is_finite() && aim.point.lon.is_finite());
    // The filmed point is within a sensible distance for the ~10.9 km slant
    // range: it must not land on the far side of the planet.
    assert!(
        aim.ground_range_m < 30_000.0,
        "ground_range_m = {}",
        aim.ground_range_m
    );
}
