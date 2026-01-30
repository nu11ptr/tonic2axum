use std::{collections::HashMap, error::Error};

use flexstr::LocalStr;
use heck::ToSnakeCase as _;
use proc_macro2::{Span, TokenStream};
use prost_build::ServiceGenerator;
use prost_reflect::{DescriptorPool, DynamicMessage};
use quote::{ToTokens as _, format_ident, quote};

use crate::{
    StateType,
    http::{HttpOptions, MessageDetails, MessageHandling, MethodDetails},
    message::{ExistingMessages, Field, Message, NewMessages},
};

fn ident(name: &str) -> syn::Ident {
    syn::Ident::new(name, Span::call_site())
}

// *** ServiceTypeGenerics and ServiceType ***

pub(crate) struct ServiceTypeGenerics {
    handler_route_turbofish: TokenStream,
    handler_generics: TokenStream,
    router_generics: TokenStream,
}

pub(crate) struct ServiceType {
    pub handler_type_name: TokenStream,
    pub router_type_name: TokenStream,
    pub generics: Option<ServiceTypeGenerics>,
    pub use_trait: Option<TokenStream>,
}

impl ServiceType {
    pub fn new(service_name: &str, state_type: Option<&StateType>) -> Self {
        fn make_fq_trait_name(service_name: &str) -> TokenStream {
            let service_mod_name = format_ident!("{}_server", service_name.to_snake_case());
            let service_trait_name = ident(service_name);

            quote! {
                #service_mod_name::#service_trait_name
            }
        }

        match state_type {
            // Custom type
            Some(StateType::Custom(type_)) => {
                let type_name = quote! { #type_ };
                let fq_trait_name = make_fq_trait_name(service_name);
                let use_trait = Some(quote! { use super::#fq_trait_name; });

                Self {
                    handler_type_name: type_name.clone(),
                    router_type_name: type_name,
                    generics: None,
                    use_trait,
                }
            }
            // Trait object
            Some(StateType::ArcTraitObj) => {
                let fq_trait_name = make_fq_trait_name(service_name);
                let router_type_name = quote! { std::sync::Arc<dyn #fq_trait_name> };
                let handler_type_name = quote! { Arc<dyn super::#fq_trait_name> };

                Self {
                    router_type_name,
                    handler_type_name,
                    generics: None,
                    use_trait: None,
                }
            }
            // Generic by default
            None => {
                let type_name = ident("S").into_token_stream();
                let handler_bound = make_fq_trait_name(service_name);
                let generics = ServiceTypeGenerics {
                    handler_route_turbofish: quote! {
                        ::<#type_name>
                    },
                    handler_generics: quote! {
                        <#type_name: super::#handler_bound>
                    },
                    router_generics: quote! {
                        <#type_name: #handler_bound + Clone>
                    },
                };

                Self {
                    handler_type_name: type_name.clone(),
                    router_type_name: type_name,
                    generics: Some(generics),
                    use_trait: None,
                }
            }
        }
    }

    pub fn handler_route_turbofish(&self) -> Option<&TokenStream> {
        self.generics
            .as_ref()
            .map(|generics| &generics.handler_route_turbofish)
    }

    pub fn handler_generics(&self) -> Option<&TokenStream> {
        self.generics
            .as_ref()
            .map(|generics| &generics.handler_generics)
    }

    pub fn router_generics(&self) -> Option<&TokenStream> {
        self.generics
            .as_ref()
            .map(|generics| &generics.router_generics)
    }
}

// *** FunctionParts ***

pub(crate) struct FunctionParts {
    pub path_extractor: Option<TokenStream>,
    pub query_extractor: Option<TokenStream>,
    pub body_extractor: Option<TokenStream>,
    pub request_builder: Option<TokenStream>,
}

impl FunctionParts {
    pub fn new(
        method_name: &str,
        method_details: &MethodDetails,
        input_type: &str,
        client_streaming: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let mut extracted_fields = Vec::new();

        let path_extractor =
            Self::make_path_extractor(&method_details.path_fields, &mut extracted_fields);
        let query_extractor =
            Self::make_query_extractor(&method_details.query_str, &mut extracted_fields);
        if client_streaming && !extracted_fields.is_empty() {
            return Err(format!(
                "Client streaming methods are not supported with query or path parameters: (Method: {})",
                method_name
            )
            .into());
        }
        let body_extractor = Self::make_body_extractor(
            &method_details.body,
            &mut extracted_fields,
            client_streaming,
        );
        let request_builder = Self::make_request_builder(&extracted_fields, input_type);

        Ok(Self {
            path_extractor,
            query_extractor,
            body_extractor,
            request_builder,
        })
    }

    fn make_path_extractor(
        fields: &[Field],
        extracted_fields: &mut Vec<syn::Ident>,
    ) -> Option<TokenStream> {
        if fields.is_empty() {
            None
        } else {
            let paths = fields.iter().map(|field| {
                let field_name = &field.ident;
                let field_type = &field.type_;
                extracted_fields.push(field_name.clone());

                quote! {
                    Path(#field_name): Path<#field_type>,
                }
            });
            Some(quote! {
                #(#paths)*
            })
        }
    }

    fn make_query_extractor(
        query_str: &Option<MessageDetails>,
        extracted_fields: &mut Vec<syn::Ident>,
    ) -> Option<TokenStream> {
        match query_str {
            Some(message_details) => match &message_details {
                MessageDetails {
                    type_name,
                    handling: MessageHandling::ExtractFields(fields),
                } => {
                    extracted_fields.extend(fields.iter().cloned());
                    Some(quote! {
                        Query(super::#type_name { #(#fields),* }): Query<super::#type_name>,
                    })
                }
                MessageDetails {
                    handling: MessageHandling::ExtractSingleField(_),
                    ..
                } => unreachable!(),
                MessageDetails {
                    type_name,
                    handling: MessageHandling::VerbatimRequest,
                } => Some(quote! {
                    req__: Query<super::#type_name>,
                }),
            },
            None => None,
        }
    }

    fn make_body_extractor(
        body: &Option<MessageDetails>,
        extracted_fields: &mut Vec<syn::Ident>,
        client_streaming: bool,
    ) -> Option<TokenStream> {
        match body {
            Some(message_details) => match &message_details {
                MessageDetails {
                    type_name,
                    handling: MessageHandling::ExtractFields(fields),
                } => {
                    extracted_fields.extend(fields.iter().cloned());
                    Some(quote! {
                        Json(super::#type_name { #(#fields),* }): Json<super::#type_name>,
                    })
                }
                MessageDetails {
                    type_name,
                    handling: MessageHandling::ExtractSingleField(field),
                } => {
                    extracted_fields.push(field.clone());
                    Some(quote! {
                        Json(super::#type_name { #field }): Json<super::#type_name>,
                    })
                }
                MessageDetails {
                    type_name,
                    handling: MessageHandling::VerbatimRequest,
                } if client_streaming => Some(quote! {
                    req__: JsonLines<super::#type_name>,
                }),
                MessageDetails {
                    type_name,
                    handling: MessageHandling::VerbatimRequest,
                } => Some(quote! {
                    req__: Json<super::#type_name>,
                }),
            },
            None => None,
        }
    }

    fn make_request_builder(
        extracted_fields: &[syn::Ident],
        input_type: &str,
    ) -> Option<TokenStream> {
        if extracted_fields.is_empty() {
            None
        } else {
            let type_name = ident(input_type);
            Some(quote! {
                let req__ = super::#type_name { #(#extracted_fields),* };
            })
        }
    }

    pub fn verbatim_request(&self) -> bool {
        self.body_extractor.is_some() && self.request_builder.is_none()
    }

    pub fn empty_request(&self) -> bool {
        self.body_extractor.is_none() && self.request_builder.is_none()
    }
}

// *** Generator ***

pub(crate) struct Generator {
    service_generator: Box<dyn ServiceGenerator>,
    state_types: HashMap<LocalStr, StateType>,
    options: HttpOptions,
    new_messages: NewMessages,
    existing_messages: ExistingMessages,
    modules: Vec<TokenStream>,
    routers: Vec<TokenStream>,
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
            new_messages: NewMessages::default(),
            existing_messages: ExistingMessages::default(),
            modules: Vec::new(),
            routers: Vec::new(),
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

        let state_type = self.state_types.get(service.name.as_str());
        let service_type = ServiceType::new(&service.name, state_type);

        let service_mod_name = format_ident!("{}_handlers", service.name.to_snake_case());

        let mut functions = Vec::with_capacity(service.methods.len());
        let mut routes = Vec::with_capacity(service.methods.len());
        for method in &service.methods {
            if let Some((function, route)) =
                self.generate_func(&service.name, method, &service_type, &service_mod_name)?
            {
                functions.push(function);
                routes.push(route);
            }
        }

        let use_trait = service_type.use_trait.as_ref();

        let module = quote! {
            /// Generated axum handlers.
            pub mod #service_mod_name {
                #![allow(unused_imports)]

                use axum::Json;
                use axum::body::Body;
                use axum::extract::{Path, Query, State};
                use axum_extra::json_lines::JsonLines;
                use std::sync::Arc;
                #use_trait

                #(#functions)*
            }
        };
        let router_func = Self::generate_router(&service.name, &service_type, routes);

        self.modules.push(module);
        self.routers.push(router_func);

        Ok(())
    }

    fn generate_struct(message: &Message) -> TokenStream {
        let fields = message.fields().iter().map(|field| {
            let field_name = &field.ident;
            let field_type = &field.type_;
            quote! {
                pub #field_name: #field_type
            }
        });
        let message_name = ident(message.name.as_ref());
        quote! {
            #[derive(serde::Deserialize)]
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
        service_mod_name: &syn::Ident,
    ) -> Result<Option<(TokenStream, TokenStream)>, Box<dyn Error>> {
        let input_type = &method.input_type;

        match self.existing_messages.get_message(input_type) {
            Some(message) => {
                if let Some(method_details) = self.options.calculate_messages(
                    service_name,
                    &method.proto_name,
                    message,
                    &self.existing_messages,
                    &mut self.new_messages,
                )? {
                    // Make the function parts from the method details
                    let func_parts = FunctionParts::new(
                        &method.name,
                        &method_details,
                        input_type,
                        method.client_streaming,
                    )?;

                    let req = if func_parts.verbatim_request() && !method.client_streaming {
                        quote! { req__.0 }
                    } else if func_parts.empty_request() && input_type == "()" {
                        quote! { () }
                    } else if func_parts.empty_request() && input_type != "()" {
                        let input_type = ident(input_type);
                        quote! { super::#input_type {} }
                    } else {
                        quote! { req__ }
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

                    let state_type = &service_type.handler_type_name;
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

                    let func = quote! {
                        #func_comments
                        pub async fn #func_name #handler_generics(
                            State(state__): State<#state_type>,
                            #path_extractor
                            #query_extractor
                            headers__: http::HeaderMap,
                            extensions__: http::Extensions,
                            #body_extractor
                        ) -> http::Response<Body> {
                            #request_builder
                            let req__ = tonic2axum::#request_func_name(headers__, extensions__, #req);
                            tonic2axum::#response_func_name(state__.#func_name(req__).await)
                        }
                    };

                    // Build the route
                    let path = method_details.path.as_ref();
                    let method = ident(&method_details.method);
                    let turbofish = service_type.handler_route_turbofish();
                    let route = quote! {
                        .route(#path, #method(#service_mod_name::#func_name #turbofish))
                    };

                    Ok(Some((func, route)))
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

    fn generate_router(
        service_name: &str,
        service_type: &ServiceType,
        routes: Vec<TokenStream>,
    ) -> TokenStream {
        let func_name = format_ident!("make_{}_router", service_name.to_snake_case());
        let type_name = &service_type.router_type_name;
        let generics = service_type.router_generics();
        let comment = format!(" Axum router for the {service_name} service");

        quote! {
            #[doc = #comment]
            pub fn #func_name #generics(state: #type_name) -> axum::Router {
                #[allow(unused_imports)]
                use axum::routing::{get, post, put, delete, patch};

                axum::Router::new()
                    #(#routes)*
                    .with_state(state)
            }
        }
    }

    fn write_code_to_buffer(&mut self, buf: &mut String) {
        // These are done last because they are gathered from each service
        let structs = self.new_messages.messages().map(Self::generate_struct);
        let modules = &self.modules;
        let routers = &self.routers;

        let file = quote! {
            #(#structs)*

            #(#modules)*

            #(#routers)*
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
