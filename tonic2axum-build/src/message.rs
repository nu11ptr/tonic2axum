use std::{collections::HashMap, error::Error, mem};

use flexstr::LocalStr;
use quote::ToTokens;

use crate::builder::GeneratorConfig;

// *** Message ***

#[derive(Clone, Debug)]
pub(crate) struct Message {
    pub name: LocalStr,
    fields: Vec<Field>,
    field_count: usize,
}

impl Message {
    pub fn new(name: LocalStr) -> Self {
        Self {
            name,
            fields: Vec::new(),
            field_count: 0,
        }
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn add_fields(&mut self, fields: Vec<Field>) {
        let field_count = fields.len();
        self.fields.extend(fields);
        self.field_count += field_count;
    }

    pub fn add_field(&mut self, field: Field) {
        self.fields.push(field);
        self.field_count += 1;
    }

    pub fn remove_field(&mut self, name: &str) -> Option<Field> {
        self.fields
            .iter()
            .position(|field| field.name == name)
            .map(|index| self.fields.remove(index))
    }

    pub fn remove_all_fields(&mut self) -> Vec<Field> {
        mem::take(&mut self.fields)
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn is_intact(&self) -> bool {
        self.field_count == self.fields.len()
    }

    pub fn intact_single_field(&self) -> bool {
        self.is_intact() && self.field_count == 1
    }

    pub fn same_fields(&self, fields: &[Field]) -> bool {
        self.fields.len() == fields.len() && self.fields.iter().all(|field| fields.contains(field))
    }
}

// *** Field ***

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub name: LocalStr,
    pub ident: syn::Ident,
    pub type_: syn::Type,
    pub type_name: LocalStr,
}

impl Field {
    pub fn new(ident: syn::Ident, type_: syn::Type) -> Self {
        let name: LocalStr = ident.to_string().into();
        let name = name.optimize();
        let type_name: LocalStr = type_.to_token_stream().to_string().into();
        let type_name = type_name.optimize();
        Self {
            name,
            ident,
            type_,
            type_name,
        }
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.type_ == other.type_
    }
}

// *** ExistingMessages ***

#[derive(Debug, Default)]
pub(crate) struct ExistingMessages(
    // Message name -> Message
    HashMap<LocalStr, Message>,
);

impl ExistingMessages {
    pub fn add_message(&mut self, message: Message) {
        self.0.insert(message.name.clone(), message);
    }

    pub fn parse_source(&mut self, src: &str) -> Result<(), Box<dyn Error>> {
        let file: syn::File = syn::parse_str(src)?;

        for item in file.items {
            if let syn::Item::Struct(struct_) = item {
                let name: LocalStr = struct_.ident.to_string().into();
                let name = name.optimize();
                let mut message = Message::new(name);

                for field in struct_.fields {
                    if let Some(ident) = field.ident {
                        let type_ = field.ty;
                        message.add_field(Field::new(ident, type_));
                    }
                }

                self.add_message(message);
            }
        }

        // Add a special message for the empty request
        self.add_message(Message::new("()".into()));

        Ok(())
    }

    pub fn get_message(&self, name: &str) -> Option<&Message> {
        self.0.get(name)
    }
}

// *** NewMessages ***

#[derive(Debug, Default)]
pub(crate) struct NewMessages {
    // Input message name -> Messages
    body_messages: HashMap<LocalStr, Vec<Message>>,
    query_messages: HashMap<LocalStr, Vec<Message>>,
}

impl NewMessages {
    fn get_or_create_message(
        messages: &mut HashMap<LocalStr, Vec<Message>>,
        input_message_name: LocalStr,
        fields: Vec<Field>,
        suffix: &str,
        type_suffix: &str,
    ) -> LocalStr {
        // Find messages for this input message name
        let messages = messages.entry(input_message_name.clone()).or_default();

        // Try to find a matching message first before creating a new one. Return the existing message if found.
        for message in messages.iter() {
            if message.same_fields(&fields) {
                return message.name.clone();
            }
        }

        // Create a new message if no matching message was found (existing message is unnumbered, so second number is 2)
        let suffix_num = messages.len() + 2;
        let name: LocalStr = format!(
            "{}{}{}{}",
            &*input_message_name, suffix, suffix_num, type_suffix
        )
        .into();
        let name = name.optimize();

        let mut message = Message::new(name);
        message.add_fields(fields);
        let ident = message.name.clone();
        messages.push(message);
        ident
    }

    pub fn get_or_create_body_message(
        &mut self,
        input_message_name: LocalStr,
        fields: Vec<Field>,
        config: &GeneratorConfig,
    ) -> LocalStr {
        Self::get_or_create_message(
            &mut self.body_messages,
            input_message_name,
            fields,
            config.body_message_suffix,
            config.type_suffix,
        )
    }

    pub fn get_or_create_query_message(
        &mut self,
        input_message_name: LocalStr,
        fields: Vec<Field>,
        config: &GeneratorConfig,
    ) -> LocalStr {
        Self::get_or_create_message(
            &mut self.query_messages,
            input_message_name,
            fields,
            config.query_message_suffix,
            config.type_suffix,
        )
    }

    pub fn body_messages(&self) -> impl Iterator<Item = &Message> {
        self.body_messages.values().flatten()
    }

    pub fn query_messages(&self) -> impl Iterator<Item = &Message> {
        self.query_messages.values().flatten()
    }
}
