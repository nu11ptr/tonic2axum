use std::{collections::HashMap, error::Error, fmt, mem};

use flexstr::{LocalStr, str::LocalStrRef};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

use crate::builder::GeneratorConfig;

// *** DocComment ***

#[derive(Clone, Debug, Default)]
pub(crate) struct DocComments(Vec<LocalStr>);

impl DocComments {
    pub fn from_struct(item: &syn::ItemStruct) -> Self {
        Self(Self::extract_doc_comments(&item.attrs))
    }

    pub fn from_field(field: &syn::Field) -> Self {
        Self(Self::extract_doc_comments(&field.attrs))
    }

    fn extract_doc_comments(attrs: &[syn::Attribute]) -> Vec<LocalStr> {
        attrs
            .iter()
            .filter_map(|attr| {
                // Check if it's a doc comment attribute
                if attr.path().is_ident("doc") {
                    // Doc comments are stored as MetaNameValue: #[doc = "value"]
                    // In syn v2, we can parse the meta directly
                    match &attr.meta {
                        syn::Meta::NameValue(meta) => {
                            // The value is an Expr, which for doc comments is always a string literal
                            match &meta.value {
                                // Remove the leading space from the doc comment
                                syn::Expr::Lit(syn::ExprLit {
                                    lit: syn::Lit::Str(lit_str),
                                    ..
                                }) => Some(LocalStrRef::from(&lit_str.value()[1..]).into_owned()),
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn to_doc_comments(&self) -> impl Iterator<Item = TokenStream> {
        self.0.iter().map(|comment| {
            // Re-add the leading space to the doc comment
            let comment = format!(" {}", comment);
            quote! { #[doc = #comment] }
        })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for DocComments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for comment in &self.0 {
            if !first {
                write!(f, "\n")?;
            }
            write!(f, "{}", comment.as_ref())?;
            first = false;
        }
        Ok(())
    }
}

impl From<&str> for DocComments {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            Self(Vec::new())
        } else if value.contains('\n') {
            Self(
                value
                    .split('\n')
                    .map(|line| LocalStrRef::from(line).into_owned())
                    .collect(),
            )
        } else {
            Self(vec![LocalStrRef::from(value).into_owned()])
        }
    }
}

// *** Message ***

#[derive(Clone, Debug)]
pub(crate) struct Message {
    pub name: LocalStr,
    pub doc_comments: DocComments,
    fields: Vec<Field>,
    field_count: usize,
}

impl Message {
    pub fn new(name: LocalStr, doc_comments: DocComments) -> Self {
        Self {
            name,
            doc_comments,
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
    pub doc_comments: DocComments,
}

impl Field {
    pub fn new(ident: syn::Ident, type_: syn::Type, doc_comments: DocComments) -> Self {
        let name: LocalStr = ident.to_string().into();
        let name = name.optimize();
        let type_name: LocalStr = type_.to_token_stream().to_string().into();
        let type_name = type_name.optimize();
        Self {
            name,
            ident,
            type_,
            type_name,
            doc_comments,
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
                let doc_comments = DocComments::from_struct(&struct_);
                let mut message = Message::new(name, doc_comments);

                for field in struct_.fields {
                    let doc_comments = DocComments::from_field(&field);
                    if let Some(ident) = field.ident {
                        let type_ = field.ty;
                        message.add_field(Field::new(ident, type_, doc_comments));
                    }
                }

                self.add_message(message);
            }
        }

        // Add a special message for the empty request
        self.add_message(Message::new("()".into(), DocComments::default()));

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
        msg_doc_comments: &DocComments,
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

        // Create a new message if no matching message was found
        let name: LocalStr = if messages.is_empty() {
            // No existing messages, so first number is 1 (and we omit number in name for #1)
            format!("{}{}{}", &*input_message_name, suffix, type_suffix).into()
        } else {
            // Since we already have an existing message, we need to add a number to the suffix
            let suffix_num = messages.len() + 1;
            format!(
                "{}{}{}{}",
                &*input_message_name, suffix, suffix_num, type_suffix
            )
            .into()
        };
        let name = name.optimize();

        let mut message = Message::new(name, msg_doc_comments.clone());
        message.add_fields(fields);
        let message_name = message.name.clone();
        messages.push(message);
        message_name
    }

    pub fn get_or_create_body_message(
        &mut self,
        input_message_name: LocalStr,
        msg_doc_comments: &DocComments,
        fields: Vec<Field>,
        config: &GeneratorConfig,
    ) -> LocalStr {
        Self::get_or_create_message(
            &mut self.body_messages,
            input_message_name,
            msg_doc_comments,
            fields,
            config.body_message_suffix,
            config.type_suffix,
        )
    }

    pub fn get_or_create_query_message(
        &mut self,
        input_message_name: LocalStr,
        msg_doc_comments: &DocComments,
        fields: Vec<Field>,
        config: &GeneratorConfig,
    ) -> LocalStr {
        Self::get_or_create_message(
            &mut self.query_messages,
            input_message_name,
            msg_doc_comments,
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
