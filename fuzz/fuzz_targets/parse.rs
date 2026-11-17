#![no_main]
use libfuzzer_sys::fuzz_target;
use misb::st0601::{Packet, Value};
use misb::{klv, st0903};

fn vmti(v: &st0903::VmtiSet<'_>) {
    let _ = (v.horizontal_fov(), v.frame_width(), v.system_name());
    for t in v.targets().flatten() {
        let _ = (t.location(), t.location_offset(), t.centroid_row_col(v.frame_width().unwrap_or(1)));
    }
}

fuzz_target!(|data: &[u8]| {
    for u in klv::split(data) {
        if let Ok((p, _)) = Packet::parse_unverified(u.raw) {
            for item in p.items().flatten() {
                match item.value() {
                    Value::Vmti(v) => vmti(&v),
                    Value::Security(s) => {
                        let _ = s.object_country_codes().map(|c| c.chars().count());
                    }
                    _ => {}
                }
            }
            let _ = (p.corners(), p.frame_center());
        }
    }
    vmti(&st0903::VmtiSet::new(data));
});
