use tempfile::tempdir;
use tonic2axum_build::{Builder, ProstConfig};

#[test]
fn test_compile() {
    let dir = tempdir().unwrap();

    let mut config = ProstConfig::new();
    config
        .out_dir(dir.path())
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
    Builder::new()
        .prost_config(config)
        .compile(&["tests/proto/test/v1/test.proto"], &["tests/proto"])
        .unwrap();

    let actual = std::fs::read_to_string(dir.path().join("test.v1.rs")).unwrap();
    let expected = std::fs::read_to_string("tests/testdata/test.v1.rs").unwrap();
    assert_eq!(actual, expected);
}
