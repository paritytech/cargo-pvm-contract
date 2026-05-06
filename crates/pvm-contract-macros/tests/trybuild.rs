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
