// Validation-only marker used to trigger a baseline package build.
// Production source and packaged module contents remain based on c4acd01.

#[test]
fn baseline_build_marker() {
    assert_eq!(2 + 2, 4);
}
