use std::{collections::HashMap, error::Error};

use flexstr::LocalStr;
use heck::ToSnakeCase as _;
use proc_macro2::{Span, TokenStream};
use prost_build::ServiceGenerator;
use prost_reflect::{DescriptorPool, DynamicMessage};
use quote::{ToTokens as _, format_ident, quote};

use crate::{
    HttpOptions, StateType,
    message::{ExistingMessages, Message, NewMessages},
};

fn ident(name: &str) -> syn::Ident {
    syn::Ident::new(name, Span::call_site())
}

// *** TonicTypeBounds and TonicType ***

pub(crate) struct TonicTypeBounds {
    handler_bound: TokenStream,
    router_bound: TokenStream,
}

pub(crate) struct TonicType {
    type_name: TokenStream,
    bounds: Option<TonicTypeBounds>,
}

impl TonicType {
    pub fn new(service_name: &str, state_type: Option<StateType>) -> Self {
        fn make_bound(service_name: &str) -> TokenStream {
            let service_mod_name = format_ident!("{}_server", service_name.to_snake_case());
            let service_trait_name = ident(service_name);

            quote! {
                super::#service_mod_name::#service_trait_name
            }
        }

        match state_type {
            // Custom type
            Some(StateType::Custom(type_)) => {
                let type_name = ident(&type_).into_token_stream();
                Self {
                    type_name,
                    bounds: None,
                }
            }
            // Trait object
            Some(StateType::ArcTraitObj) => {
                let fq_trait_name = make_bound(service_name);
                let type_name = quote! { Arc<dyn #fq_trait_name> };

                Self {
                    type_name,
                    bounds: None,
                }
            }
            // Generic by default
            None => {
                let type_name = ident("S").into_token_stream();
                let handler_bound = make_bound(service_name);
                let router_bound = quote! {
                   #handler_bound + Clone
                };

                Self {
                    type_name,
                    bounds: Some(TonicTypeBounds {
                        handler_bound,
                        router_bound,
                    }),
                }
            }
        }
    }

    pub fn handler_route_name(&self, handler_name: &syn::Ident) -> TokenStream {
        if self.bounds.is_some() {
            let type_name = &self.type_name;
            quote! {
                #handler_name::<#type_name>
            }
        } else {
            quote! {
                #handler_name
            }
        }
    }

    pub fn handler_name(&self, handler_name: &syn::Ident) -> TokenStream {
        match &self.bounds {
            Some(bounds) => {
                let type_name = &self.type_name;
                let bound = &bounds.handler_bound;
                quote! {
                    #handler_name<#type_name: #bound>
                }
            }
            None => handler_name.into_token_stream(),
        }
    }

    pub fn router_name(&self) -> TokenStream {
        match &self.bounds {
            Some(bounds) => {
                let type_name = &self.type_name;
                let bound = &bounds.router_bound;
                quote! {
                    make_router<#type_name: #bound>
                }
            }
            None => ident("make_router").into_token_stream(),
        }
    }
}

// *** Generator ***

pub(crate) struct Generator {
    service_generator: Box<dyn ServiceGenerator>,
    state_types: HashMap<LocalStr, StateType>,
    options: HttpOptions,
    existing_messages: ExistingMessages,
}

impl Generator {
    pub fn new(
        service_generator: Box<dyn ServiceGenerator>,
        bytes: Vec<u8>,
        state_types: HashMap<LocalStr, StateType>,
    ) -> Result<Self, Box<dyn Error>> {
        let dynamic_fds = Self::decode_fds(&bytes)?;
        let mut options = HttpOptions::default();
        options.parse_http_options(&dynamic_fds)?;

        Ok(Self {
            service_generator,
            state_types,
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

    fn generate_service(
        &mut self,
        service: &prost_build::Service,
        buf: &mut String,
    ) -> Result<(), Box<dyn Error>> {
        // Parse the existing messages from the buffer to start
        self.existing_messages.parse_source(buf)?;

        let mut new_messages = NewMessages::default();

        let mut methods = Vec::with_capacity(service.methods.len());
        for method in &service.methods {
            if let Some(method) =
                self.generate_function(&service.name, method, &mut new_messages)?
            {
                methods.push(method);
            }
        }

        let structs = new_messages
            .messages()
            .map(|message| self.generate_struct(message));
        let service_name = format_ident!("{}_handlers", service.name.to_snake_case());

        let module = quote! {
            #(#structs)*

            /// Generated axum handlers.
            pub mod #service_name {
                #(#methods)*
            }
        };

        buf.push_str(&module.to_string());

        Ok(())
    }

    fn generate_struct(&self, message: &Message) -> TokenStream {
        let fields = message.fields().iter().map(|field| {
            let field_name = &field.ident;
            let field_type = &field.type_;
            quote! {
                #field_name: #field_type
            }
        });
        let message_name = &message.ident;
        quote! {
            #[derive(serde::Deserialize)]
            struct #message_name {
                #(#fields),*
            }
        }
    }

    fn generate_function(
        &mut self,
        service_name: &str,
        method: &prost_build::Method,
        new_messages: &mut NewMessages,
    ) -> Result<Option<TokenStream>, Box<dyn Error>> {
        match self.existing_messages.get_message(&method.input_type) {
            Some(message) => {
                if let Some(method_details) = self.options.calculate_messages(
                    service_name,
                    &method.proto_name,
                    message,
                    &self.existing_messages,
                    new_messages,
                )? {
                    let method_name = syn::Ident::new(&method.name, proc_macro2::Span::call_site());
                    let method_comments = method.comments.leading.join("\n");

                    let method_decl = quote! {
                         #[doc = #method_comments]
                        async fn #method_name() -> http::Response<axum::body::Body> {

                        }
                    };
                    Ok(Some(method_decl))
                } else {
                    println!("No method details found");
                    Ok(None)
                }
            }
            None => Err(format!(
                "Prost generated message not found: {} for service: {} method: {}",
                method.input_type, service_name, &method.name
            )
            .into()),
        }
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
