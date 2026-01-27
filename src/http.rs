use std::{borrow::Cow, collections::HashMap, error::Error};

use flexstr::{LocalStr, str::LocalStrRef};
use prost_reflect::{DynamicMessage, Value};

use crate::message::{ExistingMessages, Field, Message, NewMessages};

const HTTP_EXTENSION_TAG: u32 = 72295728;

// *** Helper functions ***

fn get_str_field_by_name<'msg>(msg: &'msg DynamicMessage, name: &str) -> Option<Cow<'msg, str>> {
    msg.get_field_by_name(name).and_then(|v| match v {
        Cow::Borrowed(Value::String(s)) => Some(s.into()),
        Cow::Owned(Value::String(s)) => Some(s.into()),
        _ => None,
    })
}

fn get_list_field_by_name<'msg>(
    msg: &'msg DynamicMessage,
    name: &str,
) -> Option<Cow<'msg, [Value]>> {
    msg.get_field_by_name(name).and_then(|v| match v {
        Cow::Borrowed(Value::List(s)) => Some(s.into()),
        Cow::Owned(Value::List(s)) => Some(s.into()),
        _ => None,
    })
}

// *** MessageHandling ***

pub(crate) enum MessageHandling {
    VerbatimRequest,
    VerbatimBinding(syn::Ident),
    ExtractFields,
}

impl MessageHandling {
    pub fn build_request(&self) -> bool {
        match self {
            MessageHandling::VerbatimBinding(_) | MessageHandling::ExtractFields => true,
            MessageHandling::VerbatimRequest => false,
        }
    }
}

// *** MessageDetails ***

pub(crate) struct MessageDetails {
    pub ident: syn::Ident,
    pub handling: MessageHandling,
}

impl MessageDetails {
    pub fn build_request(&self) -> bool {
        self.handling.build_request()
    }
}

// *** MethodDetails ***

pub(crate) struct MethodDetails {
    pub method: LocalStr,
    pub path: LocalStr,
    pub path_fields: Vec<Field>,
    pub query_str: Option<MessageDetails>,
    pub body: Option<MessageDetails>,
}

impl MethodDetails {
    pub fn build_request(&self) -> bool {
        !self.path_fields.is_empty()
            || self
                .query_str
                .as_ref()
                .is_some_and(|details| details.build_request())
            || self
                .body
                .as_ref()
                .is_some_and(|details| details.build_request())
    }
}

// *** HttpOption ***

#[derive(Debug, Clone)]
pub struct HttpOption {
    pub method: LocalStr,
    pub pattern: LocalStr,
    pub body: Option<LocalStr>,
}

impl HttpOption {
    pub fn build_path(&self) -> LocalStr {
        // NOTE: Since we only support simple nested variables for now, we can just return the pattern "as is".
        // If that ever changes, this method will need to be updated to parse and build the path dynamically
        self.pattern.clone()
    }

    fn parse_pattern(&self, message: &mut Message) -> Result<Vec<Field>, Box<dyn Error>> {
        let mut path_fields = Vec::new();

        // NOTE: The proto file details a much more complex syntax for the pattern, but we only support very
        // simple nested variables for now (ie. /path/{field_name}, etc.)
        for part in self.pattern.split('/') {
            if part.starts_with('{') && part.ends_with('}') {
                let field_name = &part[1..part.len() - 1];
                let field = message
                    .remove_field(field_name)
                    .ok_or(format!("Path field not found: {}", field_name))?;
                path_fields.push(field);
            }
        }

        Ok(path_fields)
    }

    fn parse_body(
        &self,
        message: &mut Message,
        existing_messages: &ExistingMessages,
        new_messages: &mut NewMessages,
    ) -> Result<Option<MessageDetails>, Box<dyn Error>> {
        match &self.body {
            // Wildcard body
            Some(body) if body == "*" => {
                // Save this _before_ we remove the fields
                let intact = message.is_intact();
                let fields = message.remove_all_fields();

                // If nothing is bound by the path and the body captures everythign else, use the message itself
                if intact {
                    Ok(Some(MessageDetails {
                        ident: message.ident.clone(),
                        handling: MessageHandling::VerbatimRequest,
                    }))
                } else {
                    // Build a new struct with the remaining fields
                    let ident = new_messages.get_or_create_message(message.name.clone(), fields);
                    Ok(Some(MessageDetails {
                        ident,
                        handling: MessageHandling::ExtractFields,
                    }))
                }
            }
            // Single field body
            Some(body) => {
                // Save this _before_ we remove the field
                let intact_single_field = message.intact_single_field();

                let field = message
                    .remove_field(body.as_ref())
                    .ok_or(format!("Field not found: {}", body))?;

                // Is the field a nested message?
                let type_name = field.type_name.as_ref();
                match existing_messages.get_message(type_name) {
                    // Yes, use the existing (nested) message
                    Some(message) => Ok(Some(MessageDetails {
                        ident: message.ident.clone(),
                        handling: MessageHandling::VerbatimBinding(field.ident.clone()),
                    })),
                    // No, but this is the only field it has, so use the message itself
                    None if intact_single_field => Ok(Some(MessageDetails {
                        ident: message.ident.clone(),
                        handling: MessageHandling::VerbatimRequest,
                    })),
                    // No, but it is either not intact or has multiple fields, so we need to build a new single field struct
                    None => {
                        let ident =
                            new_messages.get_or_create_message(message.name.clone(), vec![field]);
                        Ok(Some(MessageDetails {
                            ident,
                            handling: MessageHandling::ExtractFields,
                        }))
                    }
                }
            }
            // No body
            None => Ok(None),
        }
    }

    fn parse_query_str(
        &self,
        message: &mut Message,
        new_messages: &mut NewMessages,
    ) -> Option<MessageDetails> {
        if message.is_empty() {
            // No fields left, so no query struct is needed
            None
        } else if message.is_intact() {
            // Use the message itself
            Some(MessageDetails {
                ident: message.ident.clone(),
                handling: MessageHandling::VerbatimRequest,
            })
        } else {
            // Build a new struct with the remaining fields
            let fields = message.remove_all_fields();
            let ident = new_messages.get_or_create_message(message.name.clone(), fields);
            Some(MessageDetails {
                ident,
                handling: MessageHandling::ExtractFields,
            })
        }
    }

    pub fn parse(
        &self,
        message: &Message,
        message_fields: &ExistingMessages,
        new_messages: &mut NewMessages,
    ) -> Result<MethodDetails, Box<dyn Error>> {
        let mut message = message.clone();
        let path_fields = self.parse_pattern(&mut message)?;
        let body = self.parse_body(&mut message, message_fields, new_messages)?;
        let query_str = self.parse_query_str(&mut message, new_messages);

        Ok(MethodDetails {
            method: self.method.clone(),
            path: self.build_path(),
            query_str,
            path_fields,
            body,
        })
    }
}

// *** HttpOptions ***

#[derive(Debug, Default)]
pub struct HttpOptions(
    // Service name -> Method name -> HttpOption
    HashMap<LocalStr, HashMap<LocalStr, HttpOption>>,
);

impl HttpOptions {
    pub fn calculate_messages(
        &mut self,
        service_name: &str,
        method_name: &str,
        message: &Message,
        existing_messages: &ExistingMessages,
        new_messages: &mut NewMessages,
    ) -> Result<Option<MethodDetails>, Box<dyn Error>> {
        match self.get_http_options(service_name, method_name) {
            Some(option) => Ok(Some(option.parse(
                message,
                existing_messages,
                new_messages,
            )?)),
            None => Ok(None),
        }
    }

    pub fn parse_http_options(
        &mut self,
        fds_dynamic: &DynamicMessage,
    ) -> Result<(), Box<dyn Error>> {
        // SINGLE PASS: Iterate once through files, services, and methods
        if let Some(files) = get_list_field_by_name(fds_dynamic, "file") {
            for file in files.iter() {
                let file_msg = file.as_message().ok_or("Invalid file message")?;

                if let Some(services) = get_list_field_by_name(file_msg, "service") {
                    for service in services.iter() {
                        let service_msg = service.as_message().ok_or("Invalid service message")?;
                        let service_name: LocalStrRef = get_str_field_by_name(service_msg, "name")
                            .ok_or("Invalid service name")?
                            .into();
                        let service_name = service_name.into_owned().optimize();

                        let mut method_cache = HashMap::new();

                        if let Some(methods) = get_list_field_by_name(service_msg, "method") {
                            for method in methods.iter() {
                                let method_msg =
                                    method.as_message().ok_or("Invalid method message")?;
                                let method_name: LocalStrRef =
                                    get_str_field_by_name(method_msg, "name")
                                        .ok_or("Invalid method name")?
                                        .into();
                                let method_name = method_name.into_owned().optimize();

                                // Extract options from extensions (Tag 72295728)
                                if let Some(opts) = Self::extract_options(method_msg) {
                                    method_cache.insert(method_name, opts);
                                }
                            }
                        }
                        self.0.insert(service_name, method_cache);
                    }
                }
            }
        }

        Ok(())
    }

    fn extract_options(method_msg: &DynamicMessage) -> Option<HttpOption> {
        let options_msg = method_msg.get_field_by_name("options")?;
        let options_msg = options_msg.as_message()?;

        // Find specifically decoded extensions in prost-reflect 0.16
        for (ext_desc, ext_value) in options_msg.extensions() {
            if ext_desc.number() == HTTP_EXTENSION_TAG {
                let http_rule = ext_value.as_message()?;
                let mut method = LocalStr::empty();
                let mut pattern = LocalStr::empty();
                let mut body = None;

                for (field, value) in http_rule.fields() {
                    match field.name() {
                        "get" | "post" | "put" | "delete" | "patch" => {
                            method = LocalStr::from_owned(field.name().to_uppercase()).optimize();
                            pattern = LocalStrRef::from_borrowed(value.as_str()?).into_owned();
                        }
                        "body" => {
                            body = Some(LocalStrRef::from_borrowed(value.as_str()?).into_owned())
                        }
                        _ => {}
                    }
                }

                if !method.is_empty() {
                    return Some(HttpOption {
                        method,
                        pattern,
                        body,
                    });
                }
            }
        }

        None
    }

    fn get_http_options(&self, service_name: &str, method_name: &str) -> Option<&HttpOption> {
        self.0.get(service_name)?.get(method_name)
    }
}
