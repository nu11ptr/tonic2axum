use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use flexstr::LocalStr;
use flexstr::str::LocalStrRef;
use prost_build::ServiceGenerator;
use prost_reflect::prost_types::FileDescriptorSet;

use crate::codegen::Generator;

const DEFAULT_FDS_FILE_NAME: &str = "fds.bin";

pub enum StateType {
    Custom(LocalStr),
    ArcTraitObj,
}

pub struct Builder {
    fds_path: Option<PathBuf>,
    state_types: HashMap<LocalStr, StateType>,
    prost_config: Option<prost_build::Config>,
    tonic_builder: Option<tonic_prost_build::Builder>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            fds_path: None,
            state_types: HashMap::new(),
            prost_config: None,
            tonic_builder: None,
        }
    }

    pub fn file_descriptor_set_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.fds_path = Some(path.into());
        self
    }

    pub fn custom_state_type(
        mut self,
        service_name: impl AsRef<str>,
        state_type: impl AsRef<str>,
    ) -> Self {
        let name: LocalStrRef = service_name.as_ref().into();
        let type_: LocalStrRef = state_type.as_ref().into();
        self.state_types
            .insert(name.into_owned(), StateType::Custom(type_.into_owned()));
        self
    }

    pub fn arc_trait_obj_state_type(mut self, service_name: impl AsRef<str>) -> Self {
        let name: LocalStrRef = service_name.as_ref().into();
        self.state_types
            .insert(name.into_owned(), StateType::ArcTraitObj);
        self
    }

    pub fn prost_config(mut self, config: prost_build::Config) -> Self {
        self.prost_config = Some(config);
        self
    }

    pub fn tonic_builder(mut self, builder: tonic_prost_build::Builder) -> Self {
        self.tonic_builder = Some(builder);
        self
    }

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
            self.prost_config = Some(prost_build::Config::new());
        }

        let fds_path = self.fds_path.as_ref().unwrap();
        let prost_config = self.prost_config.as_mut().unwrap();

        prost_config.file_descriptor_set_path(fds_path);
        let fds = prost_config.load_fds(protos, includes)?;
        let bytes = std::fs::read(fds_path)?;
        Ok((fds, bytes))
    }

    pub fn generate_code(
        mut self,
        fds: FileDescriptorSet,
        fds_bytes: Vec<u8>,
    ) -> Result<(), Box<dyn Error>> {
        if self.prost_config.is_none() {
            self.prost_config = Some(prost_build::Config::new());
        }

        if self.tonic_builder.is_none() {
            self.tonic_builder = Some(tonic_prost_build::configure());
        }

        let prost_config = self.prost_config.as_mut().unwrap();
        let tonic_builder = self.tonic_builder.unwrap();

        let service_generator =
            Self::make_service_generator(tonic_builder, fds_bytes, self.state_types)?;
        prost_config.service_generator(service_generator);
        prost_config.compile_fds(fds)?;
        Ok(())
    }

    fn make_service_generator(
        tonic_builder: tonic_prost_build::Builder,
        fds_bytes: Vec<u8>,
        state_types: HashMap<LocalStr, StateType>,
    ) -> Result<Box<dyn ServiceGenerator>, Box<dyn Error>> {
        Ok(Box::new(Generator::new(
            tonic_builder.service_generator(),
            fds_bytes,
            state_types,
        )?))
    }
}
