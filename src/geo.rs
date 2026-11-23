//! Where the camera is pointed: turn ST 0601 pointing metadata into a point
//! on the ground.
//!
//! An ST 0601 packet carries where the sensor is and where it is looking. The
//! operational question a counter-UAS operator, an airfield or prison security
//! post, or an investigator asks is the other end of that line: what patch of
//! ground is this camera filming? [`aimpoint`](crate::st0601::Packet::aimpoint)
//! answers it with a latitude and longitude, and
//! [`footprint`](crate::st0601::Packet::footprint) with the four ground corners
//! of the image.
//!
//! Two sources feed the answer, in order of trust:
//!
//! * The sensor's own geolocation: Frame Center (ST 0601 tags 23/24/25) or,
//!   failing that, Target Location (tags 40/41/42). The onboard computer has
//!   already intersected the line of sight with its terrain model; this is what
//!   an operator should believe when it is present.
//! * A projection from the sensor position, pointing angles and slant range,
//!   for packets that omit the frame center but still report where the gimbal
//!   is aimed. See [`project`].
//!
//! The geodesy runs in Earth-Centred Earth-Fixed (ECEF) coordinates on the
//! WGS84 ellipsoid, so there is no flat-earth or local-tangent-plane error:
//! the sensor position becomes an ECEF vector, the line of sight an ECEF unit
//! vector, the target `sensor + range * lineofsight`, and the result is turned
//! back into latitude and longitude with Bowring's closed-form inverse. WGS84
//! constants are from NIMA TR8350.2; the ellipsoid-to-geodetic inverse is
//! Bowring (1976), Transformation from spatial to geographical coordinates.

use crate::GeoPoint;

/// WGS84 semi-major axis, metres (NIMA TR8350.2).
const WGS84_A: f64 = 6_378_137.0;
/// WGS84 first eccentricity squared (NIMA TR8350.2, e^2 = f(2 - f)).
const WGS84_E2: f64 = 6.694_379_990_14e-3;
/// Mean Earth radius for great-circle range, metres (IUGG R1).
const EARTH_R: f64 = 6_371_008.8;

/// Where an aimpoint came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AimSource {
    /// Frame Center (ST 0601 tags 23/24, elevation tag 25): the sensor's own
    /// geolocation of the image centre.
    FrameCenter,
    /// Target Location (ST 0601 tags 40/41, elevation tag 42): the sensor's own
    /// geolocation of the tracked point.
    TargetLocation,
    /// Projected from sensor position, pointing angles and slant range, because
    /// the packet carried no frame-centre or target geolocation.
    Projected,
}

/// The point on the ground the camera is filming, and how far and in which
/// direction it is from the aircraft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aimpoint {
    /// The ground point the sensor is looking at.
    pub point: GeoPoint,
    /// Which ST 0601 data produced it.
    pub source: AimSource,
    /// Great-circle distance from the sensor's ground track to the aimpoint,
    /// metres. This is how far downrange the filmed ground is.
    pub ground_range_m: f64,
    /// True bearing from the sensor to the aimpoint, degrees clockwise from
    /// north: the compass direction the camera is looking.
    pub bearing_deg: f64,
}

/// The pointing geometry pulled out of an ST 0601 packet, in degrees and
/// metres, for [`project`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointing {
    /// Sensor position: latitude, longitude, height. Height should be above the
    /// WGS84 ellipsoid; Sensor True Altitude (tag 15, MSL) is accepted with a
    /// geoid-undulation caveat (see [`project`]).
    pub sensor: GeoPoint,
    /// Platform heading, degrees true (tag 5).
    pub heading_deg: f64,
    /// Platform pitch, degrees, nose up positive (tag 6 or 90).
    pub pitch_deg: f64,
    /// Platform roll, degrees, right wing down positive (tag 7 or 91).
    pub roll_deg: f64,
    /// Sensor azimuth relative to the platform longitudinal axis, degrees
    /// clockwise viewed from above (tag 18).
    pub rel_azimuth_deg: f64,
    /// Sensor elevation relative to the platform, degrees, below the
    /// longitudinal-transverse plane negative (tag 19).
    pub rel_elevation_deg: f64,
    /// Slant range from sensor to the filmed ground, metres (tag 21).
    pub slant_range_m: f64,
}

/// Direction cosine matrix for aerospace yaw-pitch-roll (3-2-1), mapping a
/// vector in the platform body frame (x forward, y right, z down) into the
/// local North-East-Down frame. Rows are N, E, D.
fn body_to_ned(yaw: f64, pitch: f64, roll: f64) -> [[f64; 3]; 3] {
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let (cp, sp) = (pitch.cos(), pitch.sin());
    let (cr, sr) = (roll.cos(), roll.sin());
    [
        [cp * cy, sr * sp * cy - cr * sy, cr * sp * cy + sr * sy],
        [cp * sy, sr * sp * sy + cr * cy, cr * sp * sy - sr * cy],
        [-sp, sr * cp, cr * cp],
    ]
}

fn mat_vec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Geodetic latitude, longitude, height to ECEF, metres.
fn geodetic_to_ecef(lat_deg: f64, lon_deg: f64, height: f64) -> [f64; 3] {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let n = WGS84_A / (1.0 - WGS84_E2 * lat.sin().powi(2)).sqrt();
    [
        (n + height) * lat.cos() * lon.cos(),
        (n + height) * lat.cos() * lon.sin(),
        (n * (1.0 - WGS84_E2) + height) * lat.sin(),
    ]
}

/// ECEF to geodetic latitude, longitude (degrees) and height (metres) by
/// Bowring's 1976 closed-form method. Accurate to well under a millimetre for
/// any height a drone flies at.
fn ecef_to_geodetic(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let b = WGS84_A * (1.0 - WGS84_E2).sqrt();
    let ep2 = (WGS84_A * WGS84_A - b * b) / (b * b);
    let p = (x * x + y * y).sqrt();
    let lon = y.atan2(x);
    if p < 1e-9 {
        // On the polar axis.
        let lat = if z >= 0.0 {
            core::f64::consts::FRAC_PI_2
        } else {
            -core::f64::consts::FRAC_PI_2
        };
        let height = z.abs() - b;
        return (lat.to_degrees(), lon.to_degrees(), height);
    }
    let theta = (z * WGS84_A).atan2(p * b);
    let lat =
        (z + ep2 * b * theta.sin().powi(3)).atan2(p - WGS84_E2 * WGS84_A * theta.cos().powi(3));
    let n = WGS84_A / (1.0 - WGS84_E2 * lat.sin().powi(2)).sqrt();
    let height = p / lat.cos() - n;
    (lat.to_degrees(), lon.to_degrees(), height)
}

/// Local East, North, Up unit vectors in ECEF at a geodetic position.
fn enu_basis(lat_deg: f64, lon_deg: f64) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let east = [-lon.sin(), lon.cos(), 0.0];
    let north = [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()];
    let up = [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()];
    (east, north, up)
}

/// Great-circle distance (metres) and initial true bearing (degrees) from `a`
/// to `b` on a sphere of mean Earth radius.
fn range_bearing(a: GeoPoint, b: GeoPoint) -> (f64, f64) {
    let (la1, la2) = (a.lat.to_radians(), b.lat.to_radians());
    let dlo = (b.lon - a.lon).to_radians();
    let dla = la2 - la1;
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    let range = 2.0 * EARTH_R * h.sqrt().min(1.0).asin();
    let y = dlo.sin() * la2.cos();
    let x = la1.cos() * la2.sin() - la1.sin() * la2.cos() * dlo.cos();
    let bearing = y.atan2(x).to_degrees().rem_euclid(360.0);
    (range, bearing)
}

/// Project the pointing geometry to the ground point the camera is filming.
///
/// The line of sight is built in the platform body frame from the gimbal
/// angles, rotated into local North-East-Down by the platform attitude
/// (heading, pitch, roll), then into ECEF, and followed `slant_range_m` metres
/// from the sensor. No terrain model is used: the slant range (ST 0601 tag 21)
/// already encodes the distance to the ground the sensor measured, so the
/// result is the point at that range along the pointing vector.
///
/// Returns `None` if the geometry is not finite, the slant range is negative,
/// or the sensor has no height.
///
/// Honest limits, for the operator reading the number:
///
/// * The sensor height should be above the WGS84 ellipsoid (tag 75 or 104).
///   If only Sensor True Altitude (tag 15, above mean sea level) is available,
///   the aimpoint is off by roughly the geoid undulation projected along the
///   look angle.
/// * The angles are taken as exact. Real gimbals have boresight and timing
///   error; a tenth of a degree at 10 km is about 17 m on the ground.
/// * When the packet carries Frame Center or Target Location, prefer those
///   (the sensor already solved the terrain intersection). Use this projection
///   only as the fallback it is meant to be.
pub fn project(p: &Pointing) -> Option<GeoPoint> {
    let height = p.sensor.height?;
    let vals = [
        p.sensor.lat,
        p.sensor.lon,
        height,
        p.heading_deg,
        p.pitch_deg,
        p.roll_deg,
        p.rel_azimuth_deg,
        p.rel_elevation_deg,
        p.slant_range_m,
    ];
    if vals.iter().any(|v| !v.is_finite()) || p.slant_range_m < 0.0 {
        return None;
    }
    let az = p.rel_azimuth_deg.to_radians();
    let el = p.rel_elevation_deg.to_radians();
    // Line of sight in the body frame: x forward, y right, z down. Elevation
    // below the horizontal is negative, so a downward look has +z (down).
    let los_body = [el.cos() * az.cos(), el.cos() * az.sin(), -el.sin()];
    let c = body_to_ned(
        p.heading_deg.to_radians(),
        p.pitch_deg.to_radians(),
        p.roll_deg.to_radians(),
    );
    let los_ned = mat_vec(&c, los_body);
    // North-East-Down to local East-North-Up.
    let (e, n, u) = (los_ned[1], los_ned[0], -los_ned[2]);
    let (east, north, up) = enu_basis(p.sensor.lat, p.sensor.lon);
    let s = geodetic_to_ecef(p.sensor.lat, p.sensor.lon, height);
    let r = p.slant_range_m;
    let t = [
        s[0] + r * (east[0] * e + north[0] * n + up[0] * u),
        s[1] + r * (east[1] * e + north[1] * n + up[1] * u),
        s[2] + r * (east[2] * e + north[2] * n + up[2] * u),
    ];
    let (lat, lon, h) = ecef_to_geodetic(t[0], t[1], t[2]);
    Some(GeoPoint {
        lat,
        lon,
        height: Some(h),
    })
}

/// Build an [`Aimpoint`] (point plus range and bearing from the sensor) from a
/// ground point and the sensor position.
pub(crate) fn aimpoint_from(sensor: GeoPoint, point: GeoPoint, source: AimSource) -> Aimpoint {
    let (ground_range_m, bearing_deg) = range_bearing(sensor, point);
    Aimpoint {
        point,
        source,
        ground_range_m,
        bearing_deg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b} (tol {tol})");
    }

    #[test]
    fn ecef_roundtrip() {
        for &(lat, lon, h) in &[
            (54.681, -110.168, 1532.0),
            (0.0, 0.0, 0.0),
            (-33.86, 151.20, 58.0),
            (78.9, -11.2, 20000.0),
        ] {
            let e = geodetic_to_ecef(lat, lon, h);
            let (la, lo, hh) = ecef_to_geodetic(e[0], e[1], e[2]);
            close(la, lat, 1e-9);
            close(lo, lon, 1e-9);
            close(hh, h, 1e-6);
        }
    }

    #[test]
    fn nadir_look_lands_straight_below() {
        // Camera pointed straight down (relative elevation -90): the ground
        // point is directly under the sensor, one slant range lower.
        let sensor = GeoPoint {
            lat: 40.0,
            lon: -75.0,
            height: Some(3000.0),
        };
        let p = Pointing {
            sensor,
            heading_deg: 137.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            rel_azimuth_deg: 210.0,
            rel_elevation_deg: -90.0,
            slant_range_m: 3000.0,
        };
        let g = project(&p).unwrap();
        close(g.lat, 40.0, 1e-7);
        close(g.lon, -75.0, 1e-7);
        close(g.height.unwrap(), 0.0, 0.05);
    }

    #[test]
    fn projection_matches_closed_form_geometry() {
        // Level platform (pitch = roll = 0) so the relative azimuth is the true
        // bearing and the relative elevation is the depression angle. The slant
        // range, azimuth and depression then define the sensor-to-target
        // vector by elementary trigonometry. Verify the ECEF projection
        // reproduces that vector exactly in the sensor's local East-North-Up
        // frame (where the flat relations hold with no curvature term):
        //   East  = slant * cos(dep) * sin(az)
        //   North = slant * cos(dep) * cos(az)
        //   Up    = -slant * sin(dep)
        let sensor = GeoPoint {
            lat: 48.0,
            lon: 11.0,
            height: Some(2500.0),
        };
        let az = 73.5_f64;
        let depression = 22.0_f64; // degrees below horizontal
        let slant = 8000.0_f64;
        let p = Pointing {
            sensor,
            heading_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            rel_azimuth_deg: az,
            rel_elevation_deg: -depression,
            slant_range_m: slant,
        };
        let g = project(&p).unwrap();

        // Express the projected vector in the sensor's ENU frame.
        let s = geodetic_to_ecef(sensor.lat, sensor.lon, sensor.height.unwrap());
        let t = geodetic_to_ecef(g.lat, g.lon, g.height.unwrap());
        let d = [t[0] - s[0], t[1] - s[1], t[2] - s[2]];
        let (east, north, up) = enu_basis(sensor.lat, sensor.lon);
        let de = d[0] * east[0] + d[1] * east[1] + d[2] * east[2];
        let dn = d[0] * north[0] + d[1] * north[1] + d[2] * north[2];
        let du = d[0] * up[0] + d[1] * up[1] + d[2] * up[2];

        let (dep, a) = (depression.to_radians(), az.to_radians());
        close(de, slant * dep.cos() * a.sin(), 1e-3);
        close(dn, slant * dep.cos() * a.cos(), 1e-3);
        close(du, -slant * dep.sin(), 1e-3);
        close((de * de + dn * dn + du * du).sqrt(), slant, 1e-3);

        // The look azimuth in the sensor's own ENU frame is exact.
        close(de.atan2(dn).to_degrees(), az, 1e-6);
        // The reported surface great-circle bearing is close, differing only by
        // meridian convergence over the range.
        let (_, bearing) = range_bearing(sensor, g);
        close(bearing, az, 0.1);
    }

    #[test]
    fn heading_and_relative_azimuth_add() {
        // The true look bearing is platform heading plus relative azimuth.
        // Split the same bearing two ways and confirm the ground point is the
        // same.
        let sensor = GeoPoint {
            lat: 48.0,
            lon: 11.0,
            height: Some(2500.0),
        };
        let base = Pointing {
            sensor,
            heading_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            rel_azimuth_deg: 100.0,
            rel_elevation_deg: -20.0,
            slant_range_m: 6000.0,
        };
        let split = Pointing {
            heading_deg: 60.0,
            rel_azimuth_deg: 40.0,
            ..base
        };
        let a = project(&base).unwrap();
        let b = project(&split).unwrap();
        close(a.lat, b.lat, 1e-9);
        close(a.lon, b.lon, 1e-9);
    }
}
