# Type Replacement Roadmap

## Current State

Type replacement in tonic2axum-build uses a post-generation AST manipulation approach: prost generates code with standard types, then `TypeReplacer` (using syn VisitMut) modifies the AST to swap types and adjust prost attributes. This lives behind the `replace_types` feature flag and depends on prettyplease + syn/visit-mut.

The current approach of marking replaced fields as `#[prost(message, required)]` is broken because prost's derive generates `encoding::message::merge()` which enters a merge_loop expecting protobuf tag-value pairs, but the wire content is raw bytes.

## Phase 1: Fork prost — `custom_string` encoding (current work)

**Goal:** Fix the broken merge path for string replacement.

**Changes:**
- Fork prost + prost-derive
- Add `encoding::custom_string` module to prost: reuses message encode/encoded_len, custom merge that reads raw bytes directly (no merge_loop)
- Add `CustomString` variant to prost-derive's scalar type system
- Update tonic2axum's TypeReplacer to emit `custom_string` instead of `message, required`
- Remove the `required` flag logic (custom_string handles proto3 implicit presence correctly)

**Replacement type contract:** Must implement `prost::Message` with `encode_raw` writing raw UTF-8, `merge` reading all remaining bytes as raw content (must override default), `encoded_len` returning byte length, `clear` resetting to empty.

## Phase 2: Fork prost — `custom_bytes` encoding

**Goal:** Same fix for bytes replacement.

**Changes:**
- Add `encoding::custom_bytes` module (identical logic to custom_string, separate name for clarity)
- Add `CustomBytes` variant to prost-derive
- Update tonic2axum's TypeReplacer for bytes fields

Since custom_string and custom_bytes have identical encoding logic (both are LengthDelimited + raw bytes), the implementation can share a private module with public aliases.

## Phase 3: Move replacement into prost-build fork

**Goal:** Eliminate tonic2axum's TypeReplacer entirely. Type replacement happens at code generation time instead of as AST post-processing.

**Changes:**
- Fork prost-build (in addition to prost + prost-derive)
- Add `Config::replace_string()` to prost-build
- During code generation, prost-build generates the replacement type directly with `#[prost(custom_string)]` attributes
- Remove from tonic2axum-build: TypeReplacer, `replace_types` feature, prettyplease dependency, syn/visit-mut dependency
- Move the `replace_string()` API from tonic2axum Builder to prost-build Config

**Benefits:**
- Cleaner architecture: replacement at generation time, not post-processing
- Simpler tonic2axum-build: no AST manipulation
- More general: any prost-build user can use type replacement

**Dependency chain:** `[patch.crates-io]` in tonic2axum's Cargo.toml replaces all transitive occurrences, so tonic-prost-build (which depends on prost-build) uses the fork automatically. Only additive API changes, so tonic-prost-build works unchanged.

## Phase 4: `custom_repeated` — container replacement for Vec

**Goal:** Replace `Vec<T>` with custom collection types (e.g., `SmallVec<T>`, `ThinVec<T>`).

**Changes:**
- Add `RepeatedAdapter` trait to prost with methods: push, clear, iterate, len
- Add `encoding::custom_repeated` module generic over RepeatedAdapter
- Update prost-derive to support custom_repeated with collection operations via the trait
- Note: `merge_repeated` is currently hardcoded to `&mut Vec<T>` — this needs generalization

**Constraint:** The replacement collection must implement the RepeatedAdapter trait.

## Phase 5: `custom_map` — container replacement for HashMap

**Goal:** Replace `HashMap<K,V>` with custom map types (e.g., `IndexMap<K,V>`, custom hasher maps).

**Changes:**
- Add `MapAdapter` trait to prost with methods: insert, clear, iterate, len
- Add `encoding::custom_map` module generic over MapAdapter
- Update prost-derive to support custom_map with map operations via the trait
- Note: map merge is currently hardcoded to `HashMap` — this needs generalization

**Constraint:** The replacement map must implement the MapAdapter trait.

## Upstream Strategy

All prost changes are designed to be additive (no existing behavior modified) and could be submitted as upstream PRs. If not accepted, the fork maintenance burden is low since the changes are isolated and unlikely to conflict with upstream development.

The `custom_string` encoding module is ~25 lines. The prost-derive changes are ~25 lines. These are "no brainer" PRs. The container replacements (Phases 4-5) are more involved and may be harder to upstream.
