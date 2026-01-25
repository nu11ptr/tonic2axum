use std::error::Error;

use prost_build::ServiceGenerator;

use crate::codegen::Generator;
use crate::http::HttpOptions;

mod codegen;
mod http;
mod message;
//mod sample;

pub struct Builder {
    service_generator: Box<dyn ServiceGenerator>,
    bytes: Vec<u8>,
}

impl Builder {
    pub fn new(service_generator: Box<dyn ServiceGenerator>, bytes: Vec<u8>) -> Self {
        Self {
            service_generator,
            bytes,
        }
    }

    pub fn into_service_generator(self) -> Result<Box<dyn ServiceGenerator>, Box<dyn Error>> {
        Ok(Box::new(Generator::new(
            self.service_generator,
            self.bytes,
        )?))
    }
}
