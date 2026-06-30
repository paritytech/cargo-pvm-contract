// trybuild `.stderr` snapshots are sensitive to both the compiler version and
// the active cfg/feature set: with `abi-gen` enabled the `#[contract]` macro
// additionally emits a `fn main()`, which collides with each fixture's own
// `fn main()` and injects a spurious `E0428` into the diagnostics. The
// committed `.stderr` files are authored for the default (non-`abi-gen`)
// configuration — the one CI runs (`cargo test -p pvm-contract-macros`) — so
// pin the UI suite to that single canonical config. Running it under
// `--all-features` would compare against a different, feature-shifted set of
// diagnostics, which is not what these snapshots assert.
#![cfg(not(feature = "abi-gen"))]

use std::path::PathBuf;

fn copy_fixtures_into_trybuild_project() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_target = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target");
    let trybuild_project = workspace_target
        .join("tests")
        .join("trybuild")
        .join("pvm-contract-macros");
    let dest_fixtures = trybuild_project.join("tests").join("ui").join("fixtures");
    std::fs::create_dir_all(&dest_fixtures).expect("create trybuild fixtures dir");

    let src_fixtures = manifest_dir.join("tests").join("ui").join("fixtures");
    for entry in std::fs::read_dir(&src_fixtures).expect("read src fixtures") {
        let entry = entry.expect("read fixture entry");
        let dest = dest_fixtures.join(entry.file_name());
        std::fs::copy(entry.path(), dest).expect("copy fixture");
    }
}

#[test]
fn ui() {
    copy_fixtures_into_trybuild_project();
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
