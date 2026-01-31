use std::error::Error;

use heck::ToSnakeCase as _;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens as _, format_ident, quote};

use crate::{
    StateType,
    http::{MessageDetails, MessageHandling, MethodDetails},
    message::Field,
};

pub(crate) fn ident(name: &str) -> syn::Ident {
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
            // Generics
            Some(StateType::Generic) => {
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
            // Trait object by default
            None => {
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
