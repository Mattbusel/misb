//! `tests/data/vmti_rust.bin` is a standalone VMTI packet from this crate's
//! encoder; the C++ tests decode it. This test keeps the file in sync
//! (`MISB_REGENERATE=1 cargo test --test vmti_file` rewrites it).
use misb::st0903::{Builder, TargetBuilder, VmtiSet};

fn build() -> Vec<u8> {
    let mut v = Builder::new(1_700_000_000_000_000);
    v.system_name("tracker-1")
        .unwrap()
        .frame_size(1280, 720)
        .unwrap()
        .fov(30.0, 17.0)
        .unwrap()
        .frame_number(4411)
        .unwrap();
    let mut a = TargetBuilder::new(7);
    a.centroid_pixel(1280 * 359 + 640)
        .unwrap()
        .confidence(87)
        .unwrap()
        .priority(1)
        .unwrap();
    a.location(38.8895, -77.0353, 12.0).unwrap();
    let mut b = TargetBuilder::new(300);
    b.centroid_row_col(100, 200)
        .unwrap()
        .location_offset(0.001, -0.002)
        .unwrap()
        .history(30)
        .unwrap();
    v.target(&a).unwrap().target(&b).unwrap();
    v.finish()
}

#[test]
fn vmti_file_matches_encoder() {
    let path = format!("{}/tests/data/vmti_rust.bin", env!("CARGO_MANIFEST_DIR"));
    let bytes = build();
    if std::env::var_os("MISB_REGENERATE").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let (set, _) = VmtiSet::parse(&bytes).unwrap();
    assert_eq!(set.targets().count(), 2);
}
