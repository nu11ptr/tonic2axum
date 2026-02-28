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
            Self::segments_match(
                target.segments.iter().skip(offset),
                pattern.segments.iter(),
            )
        }
    }

    fn segments_match<'b>(
        target: impl Iterator<Item = &'b syn::PathSegment>,
        pattern: impl Iterator<Item = &'b syn::PathSegment>,
    ) -> bool {
        target.zip(pattern).all(|(t, p)| t.ident == p.ident)
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
        assert_eq!(result, ":: core :: option :: Option < flexstr :: SharedStr >");
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
