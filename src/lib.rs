use std::collections::HashMap;
use std::error::Error;

use flexstr::LocalStr;
use flexstr::str::LocalStrRef;
use prost_build::ServiceGenerator;

use crate::codegen::Generator;
use crate::http::HttpOptions;

mod codegen;
mod http;
mod message;
//mod sample;

pub(crate) enum StateType {
    Custom(LocalStr),
    ArcTraitObj,
}

pub struct Builder {
    service_generator: Box<dyn ServiceGenerator>,
    bytes: Vec<u8>,
    state_types: HashMap<LocalStr, StateType>,
}

impl Builder {
    pub fn new(service_generator: Box<dyn ServiceGenerator>, bytes: Vec<u8>) -> Self {
        Self {
            service_generator,
            bytes,
            state_types: HashMap::new(),
        }
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

    pub fn into_service_generator(self) -> Result<Box<dyn ServiceGenerator>, Box<dyn Error>> {
        Ok(Box::new(Generator::new(
            self.service_generator,
            self.bytes,
            self.state_types,
        )?))
    }
}
