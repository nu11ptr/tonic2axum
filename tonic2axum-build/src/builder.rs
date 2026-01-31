use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use flexstr::LocalStr;
use flexstr::str::LocalStrRef;
use proc_macro2::Span;
use prost_build::ServiceGenerator;
use prost_reflect::prost_types::FileDescriptorSet;

use crate::{ProstConfig, TonicBuilder, codegen::Generator};

const DEFAULT_FDS_FILE_NAME: &str = "fds.bin";

/// The state type for a given service.
pub enum StateType {
    Custom(Box<syn::Type>),
    Generic,
}

pub(crate) struct GeneratorConfig {
    pub state_types: HashMap<LocalStr, StateType>,
    pub generate_openapi: bool,
    pub value_suffix: &'static str,
    pub type_suffix: &'static str,
    pub body_message_suffix: &'static str,
    pub query_message_suffix: &'static str,
    pub router_func_name: syn::Ident,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            state_types: HashMap::new(),
            generate_openapi: false,
            value_suffix: "__",
            type_suffix: "__",
            body_message_suffix: "Body",
            query_message_suffix: "Query",
            router_func_name: syn::Ident::new("make_router", Span::call_site()),
        }
    }
}

/// The builder for the tonic2axum code generator.
pub struct Builder {
    fds_path: Option<PathBuf>,
    prost_config: Option<ProstConfig>,
    tonic_builder: Option<TonicBuilder>,
    config: GeneratorConfig,
}

impl Builder {
    /// Create a new builder with default values.
    pub fn new() -> Self {
        Self {
            fds_path: None,
            prost_config: None,
            tonic_builder: None,
            config: GeneratorConfig::default(),
        }
    }

    /// Set a custom state type for a given service. While this is often a concrete fully qualified type name,
    /// it can also be a trait object type name. This is required, for example, when using client streaming methods,
    /// as the associated type in the generated service trait is not known.
    pub fn custom_state_type(
        mut self,
        service_name: impl AsRef<str>,
        state_type: impl AsRef<str>,
    ) -> Result<Self, Box<dyn Error>> {
        let service_name: LocalStrRef = service_name.as_ref().into();
        let state_type = state_type.as_ref();
        if service_name.is_empty() || state_type.is_empty() {
            return Err("Both service name and state type must be provided".into());
        }

        let type_: syn::Type = syn::parse_str(state_type.as_ref())?;
        self.config.state_types.insert(
            service_name.into_owned(),
            StateType::Custom(Box::new(type_)),
        );
        Ok(self)
    }

    /// Set the state type to be any type that implements the service trait.
    ///
    /// > NOTE: This is not compatible with generating OpenAPI documentation.
    pub fn generic_state_type(
        mut self,
        service_name: impl AsRef<str>,
    ) -> Result<Self, Box<dyn Error>> {
        let name: LocalStrRef = service_name.as_ref().into();
        if name.is_empty() {
            return Err("Service name cannot be empty".into());
        }

        self.config
            .state_types
            .insert(name.into_owned(), StateType::Generic);
        Ok(self)
    }

    /// Set whether to generate an OpenAPI specification (default: false).
    pub fn generate_openapi(mut self, enable: bool) -> Self {
        self.config.generate_openapi = enable;
        self
    }

    /// Set the value suffix for the generated value bindings (default: "__"). It can be empty to avoid the suffix,
    /// if you are sure the names will not conflict with any field names used in your proto messages
    /// (ie. req, headers, extensions, state).
    pub fn value_suffix(mut self, suffix: &'static str) -> Self {
        self.config.value_suffix = suffix;
        self
    }

    /// Set the type suffix for the generated struct types (default: "__"). It can be empty to avoid the suffix
    /// if you are sure the names will not conflict with any message names in your proto package.
    pub fn type_suffix(mut self, suffix: &'static str) -> Self {
        self.config.type_suffix = suffix;
        self
    }

    /// Set the body message suffix for the generated struct types (default: "Body"). It cannot be empty
    /// as that will conlfict with Prost generated struct types names.
    pub fn body_message_suffix(mut self, suffix: &'static str) -> Result<Self, Box<dyn Error>> {
        if suffix.is_empty() {
            return Err("Body message suffix cannot be empty".into());
        }
        self.config.body_message_suffix = suffix;
        Ok(self)
    }

    /// Set the query message suffix for the generated struct types (default: "Query"). It cannot be empty
    /// as that will conlfict with Prost generated struct types names.
    pub fn query_message_suffix(mut self, suffix: &'static str) -> Result<Self, Box<dyn Error>> {
        if suffix.is_empty() {
            return Err("Query message suffix cannot be empty".into());
        }
        self.config.query_message_suffix = suffix;
        Ok(self)
    }

    /// Set the router function name for the generated router functions (default: "make_router").
    /// It cannot be the same as any of the rpc method names in your proto file or it will conflict.
    pub fn router_func_name(mut self, name: impl AsRef<str>) -> Self {
        self.config.router_func_name = syn::Ident::new(name.as_ref(), Span::call_site());
        self
    }

    /// Set the path to the file descriptor set.
    pub fn file_descriptor_set_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.fds_path = Some(path.into());
        self
    }

    /// Set the prost config to customize the prost build process.
    ///
    /// > NOTE: The file descriptor set path and service generator will be overridden
    /// > by the values specified to or generated by this builder.
    pub fn prost_config(mut self, config: ProstConfig) -> Self {
        self.prost_config = Some(config);
        self
    }

    /// Set the tonic builder to customize the tonic build process.
    ///
    /// > NOTE: None of prost specific config settings will be applied.
    /// > Instead, use the [ProstConfig] directly for those settings.
    pub fn tonic_builder(mut self, builder: TonicBuilder) -> Self {
        self.tonic_builder = Some(builder);
        self
    }

    /// Compile the proto files and return the file descriptor set and its raw bytes.
    pub fn compile_protos(
        &mut self,
        protos: &[impl AsRef<Path>],
        includes: &[impl AsRef<Path>],
    ) -> Result<(FileDescriptorSet, Vec<u8>), Box<dyn Error>> {
        if self.fds_path.is_none() {
            let mut fds_path = match std::env::var("OUT_DIR") {
                Ok(out_dir) => PathBuf::from(out_dir),
                Err(_) => PathBuf::from("."),
            };
            fds_path.push(DEFAULT_FDS_FILE_NAME);
            self.fds_path = Some(fds_path);
        }

        if self.prost_config.is_none() {
            self.prost_config = Some(ProstConfig::new());
        }

        let fds_path = self.fds_path.as_ref().unwrap();
        let prost_config = self.prost_config.as_mut().unwrap();

        prost_config.file_descriptor_set_path(fds_path);
        let fds = prost_config.load_fds(protos, includes)?;
        let bytes = std::fs::read(fds_path)?;
        Ok((fds, bytes))
    }

    /// Compile the file descriptor set.
    pub fn compile_fds(
        mut self,
        fds: FileDescriptorSet,
        fds_bytes: Vec<u8>,
    ) -> Result<(), Box<dyn Error>> {
        let mut prost_config = match self.prost_config.take() {
            Some(config) => config,
            None => ProstConfig::new(),
        };

        if self.config.generate_openapi {
            prost_config.type_attribute(".", "#[derive(utoipa::ToSchema)]");
        }

        let service_generator = self.make_service_generator(fds_bytes)?;
        prost_config.service_generator(service_generator);
        prost_config.compile_fds(fds)?;
        Ok(())
    }

    /// Compile the proto files and file descriptor set.
    pub fn compile(
        mut self,
        protos: &[impl AsRef<Path>],
        includes: &[impl AsRef<Path>],
    ) -> Result<(), Box<dyn Error>> {
        let (fds, fds_bytes) = self.compile_protos(protos, includes)?;
        self.compile_fds(fds, fds_bytes)
    }

    fn make_service_generator(
        self,
        fds_bytes: Vec<u8>,
    ) -> Result<Box<dyn ServiceGenerator>, Box<dyn Error>> {
        let tonic_builder = match self.tonic_builder {
            Some(builder) => builder,
            None => tonic_prost_build::configure(),
        };
        Ok(Box::new(Generator::new(
            tonic_builder.service_generator(),
            fds_bytes,
            self.config,
        )?))
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}
