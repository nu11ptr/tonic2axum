use std::borrow::Cow;

use prost_build::ServiceGenerator;
use prost_reflect::{DescriptorPool, DynamicMessage, Value};

use std::collections::HashMap;
use std::error::Error;

const HTTP_EXTENSION_TAG: u32 = 72295728;

#[derive(Debug, Clone)]
pub struct HttpOptions {
    pub method: String,
    pub pattern: String,
    pub body: Option<String>,
}

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

pub struct Generator {
    service_generator: Box<dyn ServiceGenerator>,
    cache: HashMap<String, HashMap<String, HttpOptions>>,
    #[allow(dead_code)]
    dynamic_fds: DynamicMessage,
}

impl Generator {
    pub fn new(
        service_generator: Box<dyn ServiceGenerator>,
        bytes: &[u8],
    ) -> Result<Self, Box<dyn Error>> {
        let dynamic_fds = Self::decode_fds(bytes)?;
        let cache = Self::extract_http_options(&dynamic_fds)?;

        Ok(Self {
            service_generator,
            cache,
            dynamic_fds,
        })
    }

    fn decode_fds(bytes: &[u8]) -> Result<DynamicMessage, Box<dyn Error>> {
        let pool = DescriptorPool::decode(bytes)?;
        let fds_desc = pool
            .get_message_by_name("google.protobuf.FileDescriptorSet")
            .ok_or("Missing FileDescriptorSet schema")?;
        Ok(DynamicMessage::decode(fds_desc, bytes)?)
    }

    fn extract_http_options(
        fds_dynamic: &DynamicMessage,
    ) -> Result<HashMap<String, HashMap<String, HttpOptions>>, Box<dyn Error>> {
        let mut cache = HashMap::new();

        // SINGLE PASS: Iterate once through files, services, and methods
        if let Some(files) = get_list_field_by_name(fds_dynamic, "file") {
            for file in files.iter() {
                let file_msg = file.as_message().ok_or("Invalid file message")?;
                let pkg = get_str_field_by_name(&file_msg, "package").unwrap_or("".into());

                if let Some(services) = get_list_field_by_name(&file_msg, "service") {
                    for service in services.iter() {
                        let service_msg = service.as_message().ok_or("Invalid service message")?;
                        let srv_name = get_str_field_by_name(&service_msg, "name")
                            .ok_or("Invalid service name")?;

                        let service_name = if pkg.is_empty() {
                            srv_name.to_string()
                        } else {
                            format!("{}.{}", pkg, srv_name)
                        };
                        let mut method_cache = HashMap::new();

                        if let Some(methods) = get_list_field_by_name(&service_msg, "method") {
                            for method in methods.iter() {
                                let method_msg =
                                    method.as_message().ok_or("Invalid method message")?;
                                let method_name = get_str_field_by_name(&method_msg, "name")
                                    .ok_or("Invalid method name")?;

                                // Extract options from extensions (Tag 72295728)
                                if let Some(opts) = Self::extract_options(method_msg) {
                                    method_cache.insert(method_name.to_string(), opts);
                                }
                            }
                        }
                        cache.insert(service_name, method_cache);
                    }
                }
            }
        }

        Ok(cache)
    }

    fn extract_options(method_msg: &DynamicMessage) -> Option<HttpOptions> {
        let options_msg = method_msg.get_field_by_name("options")?;
        let options_msg = options_msg.as_message()?;

        // Find specifically decoded extensions in prost-reflect 0.16
        for (ext_desc, ext_value) in options_msg.extensions() {
            if ext_desc.number() == HTTP_EXTENSION_TAG {
                let http_rule = ext_value.as_message()?;
                let mut method = String::new();
                let mut pattern = String::new();
                let mut body = None;

                for (field, value) in http_rule.fields() {
                    match field.name() {
                        "get" | "post" | "put" | "delete" | "patch" => {
                            method = field.name().to_uppercase();
                            pattern = value.as_str()?.to_string();
                        }
                        "body" => body = Some(value.as_str()?.to_string()),
                        _ => {}
                    }
                }

                if !method.is_empty() {
                    return Some(HttpOptions {
                        method,
                        pattern,
                        body,
                    });
                }
            }
        }

        None
    }

    pub fn get_http_options(&self, service_name: &str, method_name: &str) -> Option<&HttpOptions> {
        self.cache.get(service_name)?.get(method_name)
    }
}

impl ServiceGenerator for Generator {
    fn generate(&mut self, service: prost_build::Service, buf: &mut String) {
        self.service_generator.generate(service.clone(), buf);

        println!("Generating service");
        println!("HTTP Options: {:#?}", self.cache);
    }

    fn finalize(&mut self, buf: &mut String) {
        self.service_generator.finalize(buf);
        println!("Finalizing service");
    }

    fn finalize_package(&mut self, package: &str, buf: &mut String) {
        self.service_generator.finalize_package(package, buf);
        println!("Finalizing package: {package:#?}");
    }
}
