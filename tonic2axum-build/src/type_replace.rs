use syn::visit_mut::VisitMut;

pub(crate) struct TypeReplacer<'a> {
    string_replacement: Option<&'a syn::Type>,
}

impl<'a> TypeReplacer<'a> {
    pub fn new(string_replacement: Option<&'a syn::Type>) -> Self {
        Self { string_replacement }
    }

    pub fn apply(&mut self, file: &mut syn::File) {
        self.visit_file_mut(file);
    }

    /// Match a target path against a pattern path.
    ///
    /// If the pattern has a leading `::`, an exact match is required (same segment count and
    /// leading colon). Otherwise, suffix matching is used — the pattern segments must match
    /// the last N segments of the target path.
    fn path_matches(target: &syn::Path, pattern: &syn::Path) -> bool {
        if pattern.leading_colon.is_some() {
            // Exact match: require same leading colon and segment count
            target.leading_colon.is_some()
                && target.segments.len() == pattern.segments.len()
                && Self::segments_match(target.segments.iter(), pattern.segments.iter())
        } else {
            // Suffix match: pattern segments match the tail of target segments
            if target.segments.len() < pattern.segments.len() {
                return false;
            }
            let offset = target.segments.len() - pattern.segments.len();
            Self::segments_match(target.segments.iter().skip(offset), pattern.segments.iter())
        }
    }

    fn segments_match<'b>(
        target: impl Iterator<Item = &'b syn::PathSegment>,
        pattern: impl Iterator<Item = &'b syn::PathSegment>,
    ) -> bool {
        target.zip(pattern).all(|(t, p)| t.ident == p.ident)
    }

    /// Check if a path suffix-matches `alloc::string::String`.
    fn is_string_path(path: &syn::Path) -> bool {
        let string_path: syn::Path = syn::parse_str("alloc::string::String").unwrap();
        Self::path_matches(path, &string_path)
    }
}

impl<'a> TypeReplacer<'a> {
    /// Check if a type (or any type nested in its generic arguments) matches a replacement.
    fn type_contains_replacement(&self, ty: &syn::Type) -> bool {
        if let syn::Type::Path(type_path) = ty {
            // Check direct matches
            if self.string_replacement.is_some() && Self::is_string_path(&type_path.path) {
                return true;
            }

            // Recurse into generic arguments (Vec<T>, Option<T>, HashMap<K, V>, etc.)
            for segment in &type_path.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner_ty) = arg {
                            if self.type_contains_replacement(inner_ty) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// In a `#[prost(...)]` attribute, replace `string` encoding with `custom_string`.
    ///
    /// Handles:
    /// - Bare `string` ident → `custom_string`
    /// - String literals inside map attributes (`map = "string, ..."`) → replace
    ///   `string` with `custom_string` inside the literal
    ///
    /// `custom_string` is a scalar type in prost-derive and does not need `required`.
    fn replace_prost_encoding(attr: &mut syn::Attribute) {
        if let syn::Meta::List(meta_list) = &mut attr.meta {
            let tokens: Vec<proc_macro2::TokenTree> =
                meta_list.tokens.clone().into_iter().collect();
            let mut result: Vec<proc_macro2::TokenTree> = Vec::with_capacity(tokens.len());
            let mut i = 0;

            while i < tokens.len() {
                match &tokens[i] {
                    // Bare `string` ident → `custom_string`
                    proc_macro2::TokenTree::Ident(ident) if *ident == "string" => {
                        result.push(proc_macro2::TokenTree::Ident(proc_macro2::Ident::new(
                            "custom_string",
                            ident.span(),
                        )));
                        i += 1;
                    }
                    // String literals (inside map attributes): replace `string` → `custom_string`
                    proc_macro2::TokenTree::Literal(lit) => {
                        let repr = lit.to_string();
                        if let Some(inner) =
                            repr.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                        {
                            let new_inner = inner.replace("string", "custom_string");
                            if new_inner != inner {
                                let mut new_lit = proc_macro2::Literal::string(&new_inner);
                                new_lit.set_span(lit.span());
                                result.push(proc_macro2::TokenTree::Literal(new_lit));
                                i += 1;
                                continue;
                            }
                        }
                        result.push(tokens[i].clone());
                        i += 1;
                    }
                    _ => {
                        result.push(tokens[i].clone());
                        i += 1;
                    }
                }
            }

            meta_list.tokens = result.into_iter().collect();
        }
    }
}

impl VisitMut for TypeReplacer<'_> {
    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        // Recurse into children first (depth-first)
        syn::visit_mut::visit_type_mut(self, ty);

        // Then check if this type matches any replacement
        if let syn::Type::Path(type_path) = ty {
            if let Some(replacement) = self.string_replacement {
                if Self::is_string_path(&type_path.path) {
                    *ty = replacement.clone();
                    return;
                }
            }
        }
    }

    fn visit_field_mut(&mut self, field: &mut syn::Field) {
        // Before the default walk replaces types, check if this field's type will be replaced.
        // If so, update the prost encoding attribute.
        if self.type_contains_replacement(&field.ty) {
            for attr in &mut field.attrs {
                if attr.path().is_ident("prost") {
                    Self::replace_prost_encoding(attr);
                }
            }
        }

        // Default walk: recurses into the field's type and replaces it
        syn::visit_mut::visit_field_mut(self, field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn parse_type(s: &str) -> syn::Type {
        syn::parse_str(s).unwrap()
    }

    fn replace_string_in_type(ty_str: &str, to: &str) -> String {
        let mut ty = parse_type(ty_str);
        let to_type = parse_type(to);
        let mut replacer = TypeReplacer::new(Some(&to_type));
        replacer.visit_type_mut(&mut ty);
        ty.to_token_stream().to_string()
    }

    // --- String replacement tests ---

    #[test]
    fn test_string_suffix_match() {
        let result = replace_string_in_type("::prost::alloc::string::String", "flexstr::SharedStr");
        assert_eq!(result, "flexstr :: SharedStr");
    }

    #[test]
    fn test_string_exact_match_with_leading_colon() {
        let result = replace_string_in_type("::prost::alloc::string::String", "flexstr::SharedStr");
        assert_eq!(result, "flexstr :: SharedStr");
    }

    #[test]
    fn test_string_no_match() {
        let result = replace_string_in_type("::prost::bytes::Bytes", "flexstr::SharedStr");
        assert_eq!(result, ":: prost :: bytes :: Bytes");
    }

    #[test]
    fn test_string_nested_in_option() {
        let result = replace_string_in_type(
            "::core::option::Option<::prost::alloc::string::String>",
            "flexstr::SharedStr",
        );
        assert_eq!(
            result,
            ":: core :: option :: Option < flexstr :: SharedStr >"
        );
    }

    #[test]
    fn test_string_nested_in_vec() {
        let result = replace_string_in_type(
            "::prost::alloc::vec::Vec<::prost::alloc::string::String>",
            "flexstr::SharedStr",
        );
        assert_eq!(
            result,
            ":: prost :: alloc :: vec :: Vec < flexstr :: SharedStr >"
        );
    }

    #[test]
    fn test_string_nested_in_hashmap() {
        let result = replace_string_in_type(
            "::std::collections::HashMap<::prost::alloc::string::String, ::prost::alloc::string::String>",
            "flexstr::SharedStr",
        );
        assert_eq!(
            result,
            ":: std :: collections :: HashMap < flexstr :: SharedStr , flexstr :: SharedStr >"
        );
    }
}
