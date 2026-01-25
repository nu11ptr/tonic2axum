use std::{collections::HashMap, error::Error, mem};

use flexstr::LocalStr;
use quote::{ToTokens, format_ident};

// *** Message ***

#[derive(Clone, Debug)]
pub(crate) struct Message {
    pub name: LocalStr,
    pub ident: syn::Ident,
    fields: Vec<Field>,
    field_count: usize,
}

impl Message {
    pub fn new(ident: syn::Ident) -> Self {
        let name: LocalStr = ident.to_string().into();
        let name = name.optimize();
        Self {
            name,
            ident,
            fields: Vec::new(),
            field_count: 0,
        }
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
        let name_str: LocalStr = message.ident.to_string().into();
        let name_str = name_str.optimize();
        self.0.insert(name_str, message);
    }

    pub fn parse_source(&mut self, src: &str) -> Result<(), Box<dyn Error>> {
        let file: syn::File = syn::parse_str(src)?;

        for item in file.items {
            match item {
                syn::Item::Struct(struct_) => {
                    let mut message = Message::new(struct_.ident);

                    for field in struct_.fields {
                        if let Some(ident) = field.ident {
                            let type_ = field.ty;
                            message.add_field(Field::new(ident, type_));
                        }
                    }

                    self.add_message(message);
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub fn get_message(&self, name: &str) -> Option<&Message> {
        self.0.get(name)
    }
}

// *** NewMessages ***

#[derive(Debug, Default)]
pub(crate) struct NewMessages(
    // Input message name -> Messages
    HashMap<LocalStr, Vec<Message>>,
);

impl NewMessages {
    pub fn get_or_create_message(
        &mut self,
        input_message_name: LocalStr,
        fields: Vec<Field>,
    ) -> syn::Ident {
        // Find messages for this input message name
        let messages = self
            .0
            .entry(input_message_name.clone())
            .or_insert(Vec::new());

        // Try to find a matching message first before creating a new one. Return the existing message if found.
        for message in messages.iter() {
            if message.same_fields(&fields) {
                return message.ident.clone();
            }
        }

        // Create a new message if no matching message was found (existing message is unnumbered, so second number is 2)
        let suffix_num = messages.len() + 2;
        let mut message = Message::new(format_ident!("{}{}", &*input_message_name, suffix_num));
        message.add_fields(fields);
        let ident = message.ident.clone();
        messages.push(message);
        ident
    }
}
