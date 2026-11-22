//! ST 0601 packs and lists (tags 81, 115, 116, 121, 122, 127, 128, 130,
//! 138, 139, 141, 142).
//!
//! The byte strings marked "KWIVER" are the values of those tags in the
//! first ST 0601 packet of KWIVER's `test_data/videos/h264_klv.ts`
//! (Kitware/kwiver @af3554f, BSD-3-Clause). Expected values were worked
//! out separately from the ST 1201 formula, not with this crate.

use misb::st0601::packs::*;
use misb::st0601::{Builder, Packet, Tag};
use misb::{Error, GeoPoint};

const KW_FRAME_RATE: &[u8] = &[131, 212, 96, 135, 105];
const KW_HORIZON: &[u8] = &[0, 36, 56, 0];
const KW_COMMAND: &[u8] = &[
    5, 17, 70, 108, 121, 32, 116, 111, 32, 87, 97, 121, 112, 111, 105, 110, 116, 32, 49, 0, 0, 0,
    0, 0, 0, 0, 0,
];
const KW_VERIFIED: &[u8] = &[3, 7];
const KW_ACTIVE_WAVE: &[u8] = &[1, 3];
const KW_COUNTRY: &[u8] = &[1, 14, 3, 67, 65, 78, 0, 3, 70, 82, 65];
const KW_WAVELENGTHS: &[u8] = &[13, 21, 0, 0, 7, 208, 0, 0, 15, 160, 78, 78, 73, 82];
const KW_AIRBASE: &[u8] = &[
    11, 64, 107, 194, 9, 25, 189, 165, 84, 7, 14, 0, 11, 64, 120, 60, 184, 25, 162, 146, 116, 7,
    198, 0,
];
const KW_PAYLOADS: &[u8] = &[
    3, 18, 0, 0, 15, 86, 73, 83, 32, 78, 111, 115, 101, 32, 67, 97, 109, 101, 114, 97, 21, 1, 0,
    18, 65, 67, 77, 69, 32, 86, 73, 83, 32, 77, 111, 100, 101, 108, 32, 49, 50, 51, 20, 2, 0, 17,
    65, 67, 77, 69, 32, 73, 82, 32, 77, 111, 100, 101, 108, 32, 52, 53, 54,
];
const KW_ACTIVE_PAYLOADS: &[u8] = &[11];
const KW_WAYPOINTS: &[u8] = &[
    15, 0, 0, 1, 3, 64, 113, 216, 148, 25, 189, 191, 231, 8, 152, 0, 15, 1, 0, 2, 2, 64, 113, 211,
    136, 25, 188, 206, 36, 8, 252, 0, 15, 2, 127, 255, 1, 64, 113, 227, 8, 25, 191, 44, 27, 7, 208,
    0, 15, 3, 255, 254, 0, 64, 113, 229, 175, 25, 191, 90, 167, 9, 96, 0,
];
const KW_VIEW_DOMAIN: &[u8] = &[6, 52, 128, 0, 75, 0, 0, 6, 26, 64, 0, 12, 128, 0];

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn kwiver_frame_rate() {
    let f = FrameRate::parse(KW_FRAME_RATE).unwrap();
    assert_eq!((f.numerator, f.denominator), (60_000, 1_001));
    assert!(close(f.fps().unwrap(), 59.94006, 1e-5));
    assert_eq!(FrameRate::parse(&[30]).unwrap().denominator, 1);
    assert_eq!(FrameRate::parse(&[30, 0]).unwrap().fps(), None);
    assert_eq!(FrameRate::parse(&[30, 1, 1]), Err(Error::BadLength));
    assert_eq!(f.encode().unwrap(), KW_FRAME_RATE);
}

#[test]
fn kwiver_horizon() {
    let h = HorizonPixelPack::parse(KW_HORIZON).unwrap();
    assert_eq!((h.x0, h.y0, h.x1, h.y1), (0, 36, 56, 0));
    assert_eq!((h.start, h.end), (None, None));
    assert_eq!(h.encode().unwrap(), KW_HORIZON);
    assert_eq!(HorizonPixelPack::parse(&[1, 2, 3]), Err(Error::BadLength));
}

#[test]
fn horizon_with_positions_round_trips() {
    let h = HorizonPixelPack {
        x0: 0,
        y0: 40,
        x1: 100,
        y1: 45,
        start: Some(GeoPoint {
            lat: 38.9,
            lon: -77.1,
            height: None,
        }),
        end: None,
    };
    let bytes = h.encode().unwrap();
    assert_eq!(bytes.len(), 20);
    assert_eq!(&bytes[12..16], &[0x80, 0, 0, 0]);
    let back = HorizonPixelPack::parse(&bytes).unwrap();
    let s = back.start.unwrap();
    assert!(close(s.lat, 38.9, 1e-7) && close(s.lon, -77.1, 1e-7));
    assert_eq!(back.end, None);
    let mut bad = h;
    bad.x1 = 101;
    assert!(bad.encode().is_err());
}

#[test]
fn kwiver_control_command() {
    let c = ControlCommand::parse(KW_COMMAND).unwrap();
    assert_eq!(c.id, 5);
    assert_eq!(c.text, "Fly to Waypoint 1");
    assert_eq!(c.time, Some(0));
    assert_eq!(c.encode().unwrap(), KW_COMMAND);
    let no_time = ControlCommand::parse(&KW_COMMAND[..19]).unwrap();
    assert_eq!(no_time.time, None);
    assert_eq!(
        ControlCommand::parse(&KW_COMMAND[..22]),
        Err(Error::Truncated)
    );
}

#[test]
fn kwiver_oid_lists() {
    assert_eq!(
        OidList::parse(KW_VERIFIED).unwrap().collect::<Vec<_>>(),
        [3, 7]
    );
    assert_eq!(
        OidList::parse(KW_ACTIVE_WAVE).unwrap().collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        OidList::parse(&[0x81, 0x00, 5])
            .unwrap()
            .collect::<Vec<_>>(),
        [128, 5]
    );
    assert!(OidList::parse(&[1, 0x81]).is_err());
    assert_eq!(encode_oid_list(&[3, 7]).unwrap(), KW_VERIFIED);
}

#[test]
fn kwiver_country_codes() {
    let c = CountryCodes::parse(KW_COUNTRY).unwrap();
    assert_eq!(c.coding_method, 14);
    assert_eq!(c.overflight, Some("CAN"));
    assert_eq!(c.operator, None);
    assert_eq!(c.manufacture, Some("FRA"));
    assert_eq!(c.encode().unwrap(), KW_COUNTRY);
    // Trailing codes left out.
    let short = CountryCodes::parse(&[1, 14, 3, 85, 83, 65]).unwrap();
    assert_eq!(
        (short.overflight, short.operator, short.manufacture),
        (Some("USA"), None, None)
    );
    assert_eq!(short.encode().unwrap(), [1, 14, 3, 85, 83, 65]);
    assert!(CountryCodes::parse(&[1, 14, 3, 0xFF, 0xFE, 0xFD]).is_err());
}

#[test]
fn kwiver_wavelengths() {
    let w: Vec<_> = Wavelengths::parse(KW_WAVELENGTHS).unwrap().collect();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].id, 21);
    assert_eq!((w[0].min_nm, w[0].max_nm), (1000.0, 2000.0));
    assert_eq!(w[0].name, "NNIR");
    assert_eq!(encode_wavelengths(&w).unwrap(), KW_WAVELENGTHS);
    assert!(Wavelengths::parse(&KW_WAVELENGTHS[..10]).is_err());
}

#[test]
fn kwiver_airbase_locations() {
    let a = AirbaseLocations::parse(KW_AIRBASE).unwrap();
    let t = a.takeoff.unwrap();
    let r = a.recovery.unwrap();
    assert!(close(t.lat, 38.841858983, 1e-8) && close(t.lon, -77.036784172, 1e-8));
    assert_eq!(t.height, Some(3.0));
    assert!(close(r.lat, 38.939352989, 1e-8) && close(r.lon, -77.459811211, 1e-8));
    assert_eq!(r.height, Some(95.0));
    assert_eq!(a.encode().unwrap(), KW_AIRBASE);

    // Recovery left out: same as take-off.
    let one = AirbaseLocations::parse(&KW_AIRBASE[..12]).unwrap();
    assert_eq!(one.recovery, one.takeoff);
    assert_eq!(one.encode().unwrap(), &KW_AIRBASE[..12]);
    // Unknown take-off, known recovery.
    let mut v = vec![0];
    v.extend_from_slice(&KW_AIRBASE[12..]);
    let u = AirbaseLocations::parse(&v).unwrap();
    assert_eq!(u.takeoff, None);
    assert!(u.recovery.is_some());
    assert_eq!(u.encode().unwrap(), v);
    // Location without height (8 bytes).
    let mut no_h = vec![8];
    no_h.extend_from_slice(&KW_AIRBASE[1..9]);
    assert_eq!(
        AirbaseLocations::parse(&no_h)
            .unwrap()
            .takeoff
            .unwrap()
            .height,
        None
    );
    assert_eq!(
        AirbaseLocations::parse(&[5, 1, 2, 3, 4, 5]),
        Err(Error::BadLength)
    );
}

#[test]
fn kwiver_payloads() {
    let list = PayloadList::parse(KW_PAYLOADS).unwrap();
    assert_eq!(list.count, 3);
    let p: Vec<_> = list.clone().collect();
    assert_eq!(p.len(), 3);
    assert_eq!(
        (p[0].id, p[0].payload_type, p[0].name),
        (0, 0, "VIS Nose Camera")
    );
    assert_eq!(p[1].name, "ACME VIS Model 123");
    assert_eq!((p[2].id, p[2].name), (2, "ACME IR Model 456"));
    assert_eq!(list.get(1).unwrap().name, "ACME VIS Model 123");
    assert!(list.get(9).is_none());
    assert_eq!(encode_payloads(&p).unwrap(), KW_PAYLOADS);

    let active = ActivePayloads::new(KW_ACTIVE_PAYLOADS);
    assert_eq!(active.ids().collect::<Vec<_>>(), [0, 1, 3]);
    assert!(active.is_active(1) && !active.is_active(2) && !active.is_active(64));
    assert_eq!(
        encode_active_payloads(&[0, 1, 3]).unwrap(),
        KW_ACTIVE_PAYLOADS
    );
    assert_eq!(encode_active_payloads(&[9, 0]).unwrap(), [0x02, 0x01]);
    assert_eq!(
        ActivePayloads::new(&[0x02, 0x01]).ids().collect::<Vec<_>>(),
        [0, 9]
    );
}

#[test]
fn kwiver_waypoints() {
    let w: Vec<_> = Waypoints::parse(KW_WAYPOINTS).unwrap().collect();
    assert_eq!(w.len(), 4);
    let want = [
        (0, 1, 3, 38.88942193984985, -77.03516221046448, 200.0),
        (1, 2, 2, 38.889267921447754, -77.04991817474365, 250.0),
        (2, 32767, 1, 38.88974094390869, -77.01293301582336, 100.0),
        (3, -2, 0, 38.889821887016296, -77.01009202003479, 300.0),
    ];
    for (got, (id, order, info, lat, lon, h)) in w.iter().zip(want) {
        assert_eq!((got.id, got.order, got.info), (id, order, Some(info)));
        let p = got.location.unwrap();
        assert!(close(p.lat, lat, 1e-9) && close(p.lon, lon, 1e-9));
        assert_eq!(p.height, Some(h));
    }
    assert!(w[0].manual() && w[0].ad_hoc());
    assert!(!w[1].manual() && w[1].ad_hoc());
    assert!(w[2].manual() && !w[2].ad_hoc());
    assert_eq!(encode_waypoints(&w).unwrap(), KW_WAYPOINTS);

    // Info and location are optional.
    let bare: Vec<_> = Waypoints::parse(&[3, 7, 0, 1]).unwrap().collect();
    assert_eq!(
        (bare[0].id, bare[0].order, bare[0].info, bare[0].location),
        (7, 1, None, None)
    );
    assert_eq!(encode_waypoints(&bare).unwrap(), [3, 7, 0, 1]);
    assert!(Waypoints::parse(&KW_WAYPOINTS[..20]).is_err());
}

#[test]
fn kwiver_view_domain() {
    let v = ViewDomain::parse(KW_VIEW_DOMAIN).unwrap();
    assert_eq!(
        v.azimuth,
        Some(ViewInterval {
            start: 210.0,
            range: 300.0
        })
    );
    assert_eq!(
        v.elevation,
        Some(ViewInterval {
            start: -75.0,
            range: 50.0
        })
    );
    assert_eq!(v.roll, None);
    assert_eq!(v.encode().unwrap(), KW_VIEW_DOMAIN);
    assert_eq!(ViewDomain::parse(&[3, 1, 2, 3]), Err(Error::BadLength));
    let only_roll = ViewDomain {
        azimuth: None,
        elevation: None,
        roll: Some(ViewInterval {
            start: 0.0,
            range: 90.0,
        }),
    };
    let b = only_roll.encode().unwrap();
    assert_eq!(&b[..2], &[0, 0]);
    assert_eq!(ViewDomain::parse(&b).unwrap(), only_roll);
}

#[test]
fn packet_accessors_and_builder() {
    let cmds = [
        ControlCommand {
            id: 5,
            text: "Fly to Waypoint 1",
            time: Some(0),
        },
        ControlCommand {
            id: 6,
            text: "Loiter",
            time: None,
        },
    ];
    let mut b = Builder::new(1_224_807_209_913_000);
    b.image_horizon(&HorizonPixelPack::parse(KW_HORIZON).unwrap())
        .unwrap()
        .control_command(&cmds[0])
        .unwrap()
        .control_command(&cmds[1])
        .unwrap()
        .verified_commands(&[3, 7])
        .unwrap()
        .active_wavelengths(&[1, 3])
        .unwrap()
        .country_codes(&CountryCodes::parse(KW_COUNTRY).unwrap())
        .unwrap()
        .frame_rate(&FrameRate::parse(KW_FRAME_RATE).unwrap())
        .unwrap()
        .wavelengths(
            &Wavelengths::parse(KW_WAVELENGTHS)
                .unwrap()
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .airbase_locations(&AirbaseLocations::parse(KW_AIRBASE).unwrap())
        .unwrap()
        .payloads(&PayloadList::parse(KW_PAYLOADS).unwrap().collect::<Vec<_>>())
        .unwrap()
        .active_payloads(&[0, 1, 3])
        .unwrap()
        .waypoints(&Waypoints::parse(KW_WAYPOINTS).unwrap().collect::<Vec<_>>())
        .unwrap()
        .view_domain(&ViewDomain::parse(KW_VIEW_DOMAIN).unwrap())
        .unwrap();
    let bytes = b.finish();
    let (p, _) = Packet::parse(&bytes).unwrap();

    // Every pack goes out byte for byte as KWIVER sent it.
    for (tag, want) in [
        (Tag::ImageHorizonPixelPack, KW_HORIZON),
        (Tag::ControlCommand, KW_COMMAND),
        (Tag::ControlCommandVerificationList, KW_VERIFIED),
        (Tag::ActiveWavelengthList, KW_ACTIVE_WAVE),
        (Tag::CountryCodes, KW_COUNTRY),
        (Tag::SensorFrameRatePack, KW_FRAME_RATE),
        (Tag::WavelengthsList, KW_WAVELENGTHS),
        (Tag::AirbaseLocations, KW_AIRBASE),
        (Tag::PayloadList, KW_PAYLOADS),
        (Tag::ActivePayloads, KW_ACTIVE_PAYLOADS),
        (Tag::WaypointList, KW_WAYPOINTS),
        (Tag::ViewDomain, KW_VIEW_DOMAIN),
    ] {
        assert_eq!(p.raw(tag), Some(want), "{tag:?}");
    }

    assert_eq!(p.control_commands().collect::<Vec<_>>(), cmds);
    assert_eq!(p.verified_commands().unwrap().collect::<Vec<_>>(), [3, 7]);
    assert_eq!(p.active_wavelengths().unwrap().collect::<Vec<_>>(), [1, 3]);
    assert_eq!(p.country_codes().unwrap().overflight, Some("CAN"));
    assert_eq!(p.frame_rate().unwrap().numerator, 60_000);
    assert_eq!(p.wavelengths().unwrap().next().unwrap().name, "NNIR");
    assert_eq!(
        p.airbase_locations().unwrap().takeoff.unwrap().height,
        Some(3.0)
    );
    assert_eq!(p.payloads().unwrap().count, 3);
    assert_eq!(
        p.active_payloads().unwrap().ids().collect::<Vec<_>>(),
        [0, 1, 3]
    );
    assert_eq!(p.waypoints().unwrap().count(), 4);
    assert_eq!(p.view_domain().unwrap().roll, None);
    assert_eq!(p.image_horizon().unwrap().y0, 36);

    // Absent tags.
    let empty = Builder::new(1).finish();
    let (e, _) = Packet::parse(&empty).unwrap();
    assert!(e.waypoints().is_none() && e.frame_rate().is_none());
    assert_eq!(e.control_commands().count(), 0);
}

#[test]
fn malformed_tag_reads_as_none() {
    let mut b = Builder::new(1);
    b.raw(141, &[15, 0, 0]).unwrap().raw(127, &[0x81]).unwrap();
    let bytes = b.finish();
    let (p, _) = Packet::parse(&bytes).unwrap();
    assert!(p.waypoints().is_none());
    assert!(p.frame_rate().is_none());
}

#[test]
fn no_panic_on_any_prefix_or_bit_flip() {
    let all: [&[u8]; 12] = [
        KW_FRAME_RATE,
        KW_HORIZON,
        KW_COMMAND,
        KW_VERIFIED,
        KW_ACTIVE_WAVE,
        KW_COUNTRY,
        KW_WAVELENGTHS,
        KW_AIRBASE,
        KW_PAYLOADS,
        KW_ACTIVE_PAYLOADS,
        KW_WAYPOINTS,
        KW_VIEW_DOMAIN,
    ];
    let try_all = |b: &[u8]| {
        let _ = FrameRate::parse(b);
        let _ = HorizonPixelPack::parse(b);
        let _ = ControlCommand::parse(b);
        let _ = OidList::parse(b).map(|l| l.count());
        let _ = CountryCodes::parse(b);
        let _ = Wavelengths::parse(b).map(|l| l.count());
        let _ = AirbaseLocations::parse(b);
        let _ = PayloadList::parse(b).map(|l| l.count());
        let _ = ActivePayloads::new(b).ids().count();
        let _ = Waypoints::parse(b).map(|l| l.count());
        let _ = ViewDomain::parse(b);
    };
    for v in all {
        for n in 0..=v.len() {
            try_all(&v[..n]);
        }
        for i in 0..v.len() {
            for bit in 0..8 {
                let mut m = v.to_vec();
                m[i] ^= 1 << bit;
                try_all(&m);
            }
        }
    }
}
