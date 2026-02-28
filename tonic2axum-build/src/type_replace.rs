use syn::visit_mut::VisitMut;

pub(crate) struct TypeReplacer<'a> {
    replacements: &'a [(syn::Path, syn::Type)],
}

impl<'a> TypeReplacer<'a> {
    pub fn new(replacements: &'a [(syn::Path, syn::Type)]) -> Self {
        Self { replacements }
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
}

impl<'a> TypeReplacer<'a> {
    /// Check if a type (or any type nested in its generic arguments) matches a replacement.
    fn type_contains_replacement(&self, ty: &syn::Type) -> bool {
        if let syn::Type::Path(type_path) = ty {
            if self
                .replacements
                .iter()
                .any(|(from, _)| Self::path_matches(&type_path.path, from))
            {
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

    /// In a `#[prost(...)]` attribute, replace `string` with `message`.
    ///
    /// Handles both bare idents (`#[prost(string, ...)]`) and string literals inside
    /// map attributes (`#[prost(map = "string, string", ...)]`).
    fn replace_prost_string_with_message(attr: &mut syn::Attribute) {
        if let syn::Meta::List(meta_list) = &mut attr.meta {
            meta_list.tokens = meta_list
                .tokens
                .clone()
                .into_iter()
                .map(|tt| match &tt {
                    proc_macro2::TokenTree::Ident(ident) if *ident == "string" => {
                        proc_macro2::TokenTree::Ident(proc_macro2::Ident::new(
                            "message",
                            ident.span(),
                        ))
                    }
                    proc_macro2::TokenTree::Literal(lit) => {
                        let repr = lit.to_string();
                        if let Some(inner) =
                            repr.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                        {
                            if inner.contains("string") {
                                let new_inner = inner.replace("string", "message");
                                let mut new_lit = proc_macro2::Literal::string(&new_inner);
                                new_lit.set_span(lit.span());
                                return proc_macro2::TokenTree::Literal(new_lit);
                            }
                        }
                        tt
                    }
                    _ => tt,
                })
                .collect();
        }
    }
}

impl VisitMut for TypeReplacer<'_> {
    fn visit_type_mut(&mut self, ty: &mut syn::Type) {
        // Recurse into children first (depth-first)
        syn::visit_mut::visit_type_mut(self, ty);

        // Then check if this type matches any replacement
        if let syn::Type::Path(type_path) = ty {
            for (from_path, to_type) in self.replacements {
                if Self::path_matches(&type_path.path, from_path) {
                    *ty = to_type.clone();
                    return;
                }
            }
        }
    }

    fn visit_field_mut(&mut self, field: &mut syn::Field) {
        // Before the default walk replaces types, check if this field's type will be replaced.
        // If so, update `#[prost(string, ...)]` → `#[prost(message, ...)]`.
        if self.type_contains_replacement(&field.ty) {
            for attr in &mut field.attrs {
                if attr.path().is_ident("prost") {
                    Self::replace_prost_string_with_message(attr);
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

    fn parse_path(s: &str) -> syn::Path {
        let ty: syn::TypePath = syn::parse_str(s).unwrap();
        ty.path
    }

    fn replace_in_type(ty_str: &str, from: &str, to: &str) -> String {
        let mut ty = parse_type(ty_str);
        let from_path = parse_path(from);
        let to_type = parse_type(to);
        let replacements = [(from_path, to_type)];
        let mut replacer = TypeReplacer::new(&replacements);
        replacer.visit_type_mut(&mut ty);
        ty.to_token_stream().to_string()
    }

    #[test]
    fn test_suffix_match() {
        let result = replace_in_type(
            "::prost::alloc::string::String",
            "alloc::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(result, "flexstr :: SharedStr");
    }

    #[test]
    fn test_exact_match_with_leading_colon() {
        let result = replace_in_type(
            "::prost::alloc::string::String",
            "::prost::alloc::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(result, "flexstr :: SharedStr");
    }

    #[test]
    fn test_no_match() {
        let result = replace_in_type(
            "::prost::alloc::string::String",
            "std::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(result, ":: prost :: alloc :: string :: String");
    }

    #[test]
    fn test_nested_in_option() {
        let result = replace_in_type(
            "::core::option::Option<::prost::alloc::string::String>",
            "alloc::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(
            result,
            ":: core :: option :: Option < flexstr :: SharedStr >"
        );
    }

    #[test]
    fn test_nested_in_vec() {
        let result = replace_in_type(
            "::prost::alloc::vec::Vec<::prost::alloc::string::String>",
            "alloc::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(
            result,
            ":: prost :: alloc :: vec :: Vec < flexstr :: SharedStr >"
        );
    }

    #[test]
    fn test_nested_in_hashmap() {
        let result = replace_in_type(
            "::std::collections::HashMap<::prost::alloc::string::String, ::prost::alloc::string::String>",
            "alloc::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(
            result,
            ":: std :: collections :: HashMap < flexstr :: SharedStr , flexstr :: SharedStr >"
        );
    }

    #[test]
    fn test_exact_match_no_leading_colon_no_match() {
        // Pattern with :: should not match target without ::
        let result = replace_in_type(
            "alloc::string::String",
            "::alloc::string::String",
            "flexstr::SharedStr",
        );
        assert_eq!(result, "alloc :: string :: String");
    }
}
