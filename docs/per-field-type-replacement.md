# Per-Field Type Replacement via Protobuf Custom Options

## Context

Currently, type replacement in tonic2axum-build is all-or-nothing: `replace_string("flexstr::SharedStr")` replaces **every** `String` field across all messages. There's no way to selectively replace types on specific fields. This document explores how to use protobuf3 custom options to annotate individual fields in `.proto` files, allowing per-field control over type replacement.

## Protobuf Custom Options (Recommended Approach)

### How It Works

Protobuf has a built-in extension mechanism for options at every level (file, message, field, enum, etc.). Extensions in the number range **50000–99999** are reserved for internal/organizational use.

1. **Define the option once** in a shared `.proto` file
2. **Import and use** in any proto file that needs selective replacement
3. **protoc preserves** the option in the binary `FileDescriptorSet`
4. **Other code generators ignore** options they don't recognize

### Options Proto Definition

```proto
// tonic2axum/options.proto
syntax = "proto3";
import "google/protobuf/descriptor.proto";
package tonic2axum;

extend google.protobuf.FieldOptions {
  optional string replace_type = 50000;
}
```

### Usage in Proto Files

```proto
import "tonic2axum/options.proto";

message MyRequest {
  string name = 1 [(tonic2axum.replace_type) = "flexstr::SharedStr"];
  string description = 2;  // stays as standard String
  bytes payload = 3 [(tonic2axum.replace_type) = "bytes::Bytes"];
}
```

### Why This Fits tonic2axum

- **Existing infrastructure**: `prost-reflect` + `DynamicMessage` in `generator.rs` already reads extensions from the `FileDescriptorSet` (same mechanism used for `google.api.http` at tag 72295728 in `http.rs`)
- **Standard mechanism**: This is the protobuf-sanctioned way to add metadata; same pattern as `google.api.http`, `google.api.field_behavior`, etc.
- **Validated by protoc**: Typos in the option name cause a compile error, unlike comment-based approaches

### Implementation Outline

1. **Create `tonic2axum/options.proto`** defining the `replace_type` field extension
2. **Read field options** from the `FileDescriptorSet` during code generation (in `generator.rs`, using the same `prost-reflect` pattern as HTTP option extraction)
3. **Build a field-level replacement map**: `(message_name, field_name) -> replacement_type`
4. **Modify `TypeReplacer`** (in `type_replace.rs`) to consult the map instead of doing blanket replacement
5. **Decide API surface**: per-field options could coexist with or replace the current `replace_string()` / `replace_bytes()` builder methods

### Key Files

- `tonic2axum-build/src/builder.rs` — Builder API, currently has `replace_string()` / `replace_bytes()`
- `tonic2axum-build/src/codegen/generator.rs` — FileDescriptorSet decoding, orchestrates code generation
- `tonic2axum-build/src/codegen/http.rs` — Existing example of reading extensions from descriptors
- `tonic2axum-build/src/type_replace.rs` — `TypeReplacer` visitor that does the actual AST mutation

## Alternative: Comments via SourceCodeInfo

Proto comments are preserved in the `FileDescriptorSet` via `SourceCodeInfo`. Structured comments could signal replacement:

```proto
message MyRequest {
  // @tonic2axum:replace=flexstr::SharedStr
  string name = 1;
}
```

**Pros:** No imports needed, invisible to all other tools.
**Cons:** Fragile (depends on source info preservation, comment association rules), no validation by protoc, easy to silently typo.

**Not recommended** — custom options are more robust and already align with existing infrastructure.
