use std::error::Error;

use heck::ToSnakeCase as _;
use proc_macro2::TokenStream;
use prost_build::ServiceGenerator;
use prost_reflect::{DescriptorPool, DynamicMessage};
use quote::{format_ident, quote};

use crate::{
    builder::GeneratorConfig,
    codegen::helpers::{FunctionParts, ServiceType, ValueNames, ident},
    http::HttpOptions,
    message::{ExistingMessages, Message, NewMessages},
};

pub(crate) struct Generator {
    service_generator: Box<dyn ServiceGenerator>,

    options: HttpOptions,
    new_messages: NewMessages,
    existing_messages: ExistingMessages,
    modules: Vec<TokenStream>,
    value_names: ValueNames,

    config: GeneratorConfig,
}

impl Generator {
    pub fn new(
        service_generator: Box<dyn ServiceGenerator>,
        bytes: Vec<u8>,
        config: GeneratorConfig,
    ) -> Result<Self, Box<dyn Error>> {
        let dynamic_fds = Self::decode_fds(&bytes)?;
        let mut options = HttpOptions::default();
        options.parse_http_options(&dynamic_fds)?;

        Ok(Self {
            service_generator,
            options,
            new_messages: NewMessages::default(),
            existing_messages: ExistingMessages::default(),
            modules: Vec::new(),
            value_names: ValueNames::new(config.value_suffix),
            config,
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
        buf: &str,
    ) -> Result<(), Box<dyn Error>> {
        // Parse the existing messages from the buffer to start
        self.existing_messages.parse_source(buf)?;

        let state_type = self.config.state_types.get(service.name.as_str());
        let has_trait_object_state_type = state_type.is_none();
        let service_type = ServiceType::new(&service.name, state_type);
        // This is due to the need to use turbofish for the handler function, but routes! macro doesn't support it.
        // See: https://github.com/juhaku/utoipa/issues/1234
        // In progress PR: https://github.com/juhaku/utoipa/pull/1329
        if service_type.generics.is_some() && self.config.generate_openapi {
            return Err(format!(
                "A generic service state type is not supported when generating OpenAPI documentation: (Service: {})",
                service.name
            )
            .into());
        }

        let mut handler_funcs = Vec::with_capacity(service.methods.len());
        let mut routes = Vec::with_capacity(service.methods.len());
        let mut has_client_streaming = false;

        for method in &service.methods {
            if let Some((function, route)) =
                self.generate_func(&service.name, method, &service_type)?
            {
                if method.client_streaming {
                    has_client_streaming = true;
                }
                handler_funcs.push(function);
                routes.push(route);
            }
        }

        let service_mod_name = format_ident!(
            "{}{}",
            service.name.to_snake_case(),
            self.config.service_mod_name_suffix
        );
        let use_json_lines = if has_client_streaming {
            // This is due to the need to fill in the associated type in the trait object type. This effectively
            // no longer becomes automatic as this option was intended to be. It is now equivalent to using a custom
            // state type, so just do that (either with  the trait object type + associated type or the type itself).
            if has_trait_object_state_type {
                return Err(format!(
                    "Client streaming methods are not supported when the state type is a trait object: (Service: {})",
                    service.name
                )
                .into());
            }

            Some(quote! { use axum_extra::json_lines::JsonLines; })
        } else {
            None
        };
        let use_trait = service_type.use_trait.as_ref();
        let use_routing = if self.config.generate_openapi {
            quote! {
                use utoipa_axum::routes;
                use utoipa_axum::router::OpenApiRouter;
            }
        } else {
            quote! {
                use axum::routing::{get, post, put, delete, patch};
                use axum::Router;
            }
        };
        let router_func = self.generate_router(&service.name, &service_type, routes);

        let module = quote! {
            /// Generated axum handlers and router.
            pub mod #service_mod_name {
                #![allow(unused_imports)]

                use std::sync::Arc;

                use axum::Json;
                use axum::body::Body;
                use axum::extract::{Path, Query, State};
                #use_json_lines
                #use_routing

                #use_trait

                #(#handler_funcs)*

                #router_func
            }
        };

        self.modules.push(module);

        Ok(())
    }

    fn generate_struct(&self, message: &Message, body: bool) -> TokenStream {
        let fields = message.fields().iter().map(|field| {
            let field_name = &field.ident;
            let field_type = &field.type_;
            quote! {
                pub #field_name: #field_type
            }
        });
        let message_name = ident(message.name.as_ref());

        let derive_attributes = if self.config.generate_openapi {
            if body {
                quote! { #[derive(serde::Deserialize, utoipa::ToSchema)] }
            } else {
                quote! { #[derive(serde::Deserialize, utoipa::IntoParams)] }
            }
        } else {
            quote! { #[derive(serde::Deserialize)] }
        };
        quote! {
            #derive_attributes
            pub struct #message_name {
                #(#fields),*
            }
        }
    }

    fn generate_func(
        &mut self,
        service_name: &str,
        method: &prost_build::Method,
        service_type: &ServiceType,
    ) -> Result<Option<(TokenStream, TokenStream)>, Box<dyn Error>> {
        let input_type = &method.input_type;

        match self.existing_messages.get_message(input_type) {
            Some(message) => {
                let method_details = self.options.parse(
                    service_name,
                    &method.proto_name,
                    message,
                    &self.existing_messages,
                    &mut self.new_messages,
                    &self.config,
                )?;

                match method_details {
                    Some(method_details) => {
                        let (req, headers, extensions, state) = self.value_names.names();

                        // Make the function parts from the method details
                        let func_parts = FunctionParts::new(
                            &method.name,
                            &method_details,
                            input_type,
                            method.client_streaming,
                            req,
                        )?;

                        let req = if func_parts.verbatim_request() && !method.client_streaming {
                            // Verbatim request so no need to build the request. It is a Json<T> (a tuple struct).
                            quote! { #req.0 }
                        } else if func_parts.empty_request() && input_type == "()" {
                            // Special case for the empty request which tonic replaces with unit.
                            quote! { () }
                        } else if func_parts.empty_request() && input_type != "()" {
                            // Empty message, but not the special google.protobuf.Empty message, so a struct with no fields
                            // needs to be created as there won't be a parameter for it.
                            let input_type = ident(input_type);
                            quote! { super::#input_type {} }
                        } else {
                            // Normal case, just reference the request itself directly.
                            quote! { #req }
                        };

                        let FunctionParts {
                            path_extractor,
                            query_extractor,
                            body_extractor,
                            request_builder,
                        } = func_parts;

                        let func_name = ident(&method.name);
                        let func_comments = method.comments.leading.join("\n");
                        let func_comments = if func_comments.is_empty() {
                            None
                        } else {
                            Some(quote! { #[doc = #func_comments] })
                        };

                        let state_type = &service_type.state_type_name;
                        let handler_generics = service_type.handler_generics();

                        let request_func_name = if method.client_streaming {
                            quote! { make_stream_request }
                        } else {
                            quote! { make_request }
                        };
                        let response_func_name = if method.server_streaming {
                            quote! { make_stream_response }
                        } else {
                            quote! { make_response }
                        };

                        let method = ident(&method_details.method);
                        let path = method_details.path.as_ref();
                        let path_attr = if self.config.generate_openapi {
                            Some(
                                quote! { #[utoipa::path(#method, path = #path, tag = #service_name)] },
                            )
                        } else {
                            None
                        };

                        let func = quote! {
                            #func_comments
                            #path_attr
                            pub async fn #func_name #handler_generics(
                                State(#state): State<#state_type>,
                                #path_extractor
                                #query_extractor
                                #headers: http::HeaderMap,
                                #extensions: http::Extensions,
                                #body_extractor
                            ) -> http::Response<Body> {
                                #request_builder
                                let #req = tonic2axum::#request_func_name(#headers, #extensions, #req);
                                tonic2axum::#response_func_name(#state.#func_name(#req).await)
                            }
                        };

                        // Build the route
                        let turbofish = service_type.handler_route_turbofish();
                        let route = if self.config.generate_openapi {
                            quote! { .routes(routes!(#func_name)) }
                        } else {
                            let path = method_details.path.as_ref();
                            let method = ident(&method_details.method);
                            quote! { .route(#path, #method(#func_name #turbofish)) }
                        };

                        Ok(Some((func, route)))
                    }
                    None => {
                        println!("No method details found");
                        Ok(None)
                    }
                }
            }
            None => Err(format!(
                "Prost generated message not found: {} for service: {} method: {}",
                method.input_type, service_name, &method.name
            )
            .into()),
        }
    }

    fn generate_router(
        &self,
        service_name: &str,
        service_type: &ServiceType,
        routes: Vec<TokenStream>,
    ) -> TokenStream {
        let router_func_name = &self.config.router_func_name;
        let state_type_name = &service_type.state_type_name;
        let generics = service_type.router_generics();
        let comment = format!(" Axum router for the {service_name} service");

        let router_type = if self.config.generate_openapi {
            ident("OpenApiRouter")
        } else {
            ident("Router")
        };

        quote! {
            #[doc = #comment]
            pub fn #router_func_name #generics(state: #state_type_name) -> #router_type {
                #router_type::new()
                    #(#routes)*
                    .with_state(state)
            }
        }
    }

    fn write_code_to_buffer(&mut self, buf: &mut String) {
        // These are done last because they are gathered from each service
        let body_structs = self
            .new_messages
            .body_messages()
            .map(|message| self.generate_struct(message, true));
        let query_structs = self
            .new_messages
            .query_messages()
            .map(|message| self.generate_struct(message, false));
        let modules = &self.modules;

        let file = quote! {
            #(#body_structs)*
            #(#query_structs)*

            #(#modules)*
        };

        buf.push_str(&file.to_string());
    }
}

impl ServiceGenerator for Generator {
    fn generate(&mut self, service: prost_build::Service, buf: &mut String) {
        println!("Generating service: {}", service.name);

        if let Err(e) = self.generate_service(&service, buf) {
            panic!("Failed to generate service: {e:#?}");
        }

        // Generate tonic_prost_build service code last - no need to parse the trait code
        self.service_generator.generate(service, buf);
    }

    fn finalize(&mut self, buf: &mut String) {
        println!("Finalizing service");
        self.service_generator.finalize(buf);
    }

    fn finalize_package(&mut self, package: &str, buf: &mut String) {
        println!("Finalizing package: {package:#?}");
        self.service_generator.finalize_package(package, buf);

        self.write_code_to_buffer(buf);
    }
}
