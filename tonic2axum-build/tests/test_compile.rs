#[cfg(not(feature = "cleanup-markdown"))]
mod test_compile {
    use tempfile::tempdir;
    use tonic2axum_build::{Builder, OpenApiSecurity, ProstConfig};

    #[test]
    fn test_compile_with_web_sockets() {
        let dir = tempdir().unwrap();

        let mut config = ProstConfig::new();
        config
            .out_dir(dir.path())
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
        Builder::new()
            .prost_config(config)
            .file_descriptor_set_path(dir.path().join("fds.bin"))
            .custom_state_type("StreamingTest", "crate::StreamingTest")
            .unwrap()
            .generate_web_sockets(true)
            .compile(&["tests/proto/test_ws/v1/test_ws.proto"], &["tests/proto"])
            .unwrap();

        let actual = std::fs::read_to_string(dir.path().join("test_ws.v1.rs")).unwrap();
        let expected = std::fs::read_to_string("tests/testdata/ws/test_ws.v1.rs").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compile() {
        let dir = tempdir().unwrap();

        let mut config = ProstConfig::new();
        config
            .out_dir(dir.path())
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
        Builder::new()
            .prost_config(config)
            .file_descriptor_set_path(dir.path().join("fds.bin"))
            .compile(&["tests/proto/test/v1/test.proto"], &["tests/proto"])
            .unwrap();

        let actual = std::fs::read_to_string(dir.path().join("test.v1.rs")).unwrap();
        let expected = std::fs::read_to_string("tests/testdata/test.v1.rs").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compile_with_openapi_and_web_sockets() {
        let dir = tempdir().unwrap();

        let mut config = ProstConfig::new();
        config
            .out_dir(dir.path())
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
        Builder::new()
            .prost_config(config)
            .file_descriptor_set_path(dir.path().join("fds.bin"))
            .custom_state_type("StreamingTest", "crate::StreamingTest")
            .unwrap()
            .generate_openapi(true)
            .generate_web_sockets(true)
            .compile(&["tests/proto/test_ws/v1/test_ws.proto"], &["tests/proto"])
            .unwrap();

        let actual = std::fs::read_to_string(dir.path().join("test_ws.v1.rs")).unwrap();
        let expected =
            std::fs::read_to_string("tests/testdata/openapi_ws/test_ws.v1.rs").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compile_with_web_sockets_generic_state() {
        let dir = tempdir().unwrap();

        let mut config = ProstConfig::new();
        config
            .out_dir(dir.path())
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
        Builder::new()
            .prost_config(config)
            .file_descriptor_set_path(dir.path().join("fds.bin"))
            .generic_state_type("StreamingTest")
            .unwrap()
            .generate_web_sockets(true)
            .compile(&["tests/proto/test_ws/v1/test_ws.proto"], &["tests/proto"])
            .unwrap();

        let actual = std::fs::read_to_string(dir.path().join("test_ws.v1.rs")).unwrap();
        let expected =
            std::fs::read_to_string("tests/testdata/ws_generic/test_ws.v1.rs").unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compile_with_openapi_security() {
        let dir = tempdir().unwrap();

        let mut config = ProstConfig::new();
        config
            .out_dir(dir.path())
            .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
        Builder::new()
            .prost_config(config)
            .file_descriptor_set_path(dir.path().join("fds.bin"))
            .generate_openapi(true)
            .openapi_security(OpenApiSecurity::AllServices("Bearer"))
            .compile(&["tests/proto/test/v1/test.proto"], &["tests/proto"])
            .unwrap();

        let actual = std::fs::read_to_string(dir.path().join("test.v1.rs")).unwrap();
        let expected = std::fs::read_to_string("tests/testdata/openapi/test.v1.rs").unwrap();
        assert_eq!(actual, expected);
    }
}
