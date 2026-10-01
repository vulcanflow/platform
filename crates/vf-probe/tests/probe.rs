//! Test file for the VUL-2 protection probe. Its presence alongside
//! crates/vf-probe/src/lib.rs is the lane crossing the gate must refuse.

#[test]
fn lane_of_splits_production_from_test() {
    assert_eq!(vf_probe::lane_of("crates/vf-core/src/lib.rs"), "production");
    assert_eq!(vf_probe::lane_of("crates/vf-core/tests/api.rs"), "test");
}
