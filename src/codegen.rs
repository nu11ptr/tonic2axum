use std::error::Error;

use heck::ToSnakeCase as _;
use prost_build::ServiceGenerator;
use prost_reflect::{DescriptorPool, DynamicMessage};
use quote::{format_ident, quote};

use crate::{
    HttpOptions,
    message::{ExistingMessages, NewMessages},
};

pub(crate) struct Generator {
    service_generator: Box<dyn ServiceGenerator>,
    options: HttpOptions,
    existing_messages: ExistingMessages,
}

impl Generator {
    pub fn new(
        service_generator: Box<dyn ServiceGenerator>,
        bytes: Vec<u8>,
    ) -> Result<Self, Box<dyn Error>> {
        let dynamic_fds = Self::decode_fds(&bytes)?;
        let mut options = HttpOptions::default();
        options.parse_http_options(&dynamic_fds)?;

        Ok(Self {
            service_generator,
            options,
            existing_messages: ExistingMessages::default(),
        })
    }

    fn decode_fds(bytes: &[u8]) -> Result<DynamicMessage, Box<dyn Error>> {
        let pool = DescriptorPool::decode(bytes)?;
        let fds_desc = pool
            .get_message_by_name("google.protobuf.FileDescriptorSet")
            .ok_or("Missing FileDescriptorSet schema")?;
        Ok(DynamicMessage::decode(fds_desc, bytes)?)
    }

    pub fn generate_service(
        &mut self,
        service: &prost_build::Service,
        buf: &mut String,
    ) -> Result<(), Box<dyn Error>> {
        // Parse the existing messages from the buffer to start
        self.existing_messages.parse_source(buf)?;

        let mut new_messages = NewMessages::default();

        for method in &service.methods {
            match self.existing_messages.get_message(&method.input_type) {
                Some(message) => {
                    if let Some(_http_options) = self.options.calculate_messages(
                        &service.name,
                        &method.name,
                        &message,
                        &self.existing_messages,
                        &mut new_messages,
                    )? {
                        // TODO: Generate the method definition
                    }
                }
                None => {
                    return Err(format!(
                        "Prost generated message not found: {} for service: {} method: {}",
                        method.input_type, &service.name, &method.name
                    )
                    .into());
                }
            }
        }

        let service_name = format_ident!("{}_handlers", service.name.to_snake_case());

        let module = quote! {
            pub mod #service_name {
            }
        };

        buf.push_str(&module.to_string());

        Ok(())
    }
}

impl ServiceGenerator for Generator {
    fn generate(&mut self, service: prost_build::Service, buf: &mut String) {
        println!("Generating service: {}", service.name);

        if let Err(e) = self.generate_service(&service, buf) {
            panic!("Failed to generate service: {e:#?}");
        }

        // Generate tonic_prost_build service code last
        self.service_generator.generate(service, buf);
    }

    fn finalize(&mut self, buf: &mut String) {
        println!("Finalizing service");
        self.service_generator.finalize(buf);
    }

    fn finalize_package(&mut self, package: &str, buf: &mut String) {
        println!("Finalizing package: {package:#?}");
        self.service_generator.finalize_package(package, buf);
    }
}
