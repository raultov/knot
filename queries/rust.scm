;; Tree-sitter query file for Rust entity extraction
;; Version: 0.3 (minimal - based on tree-sitter-rust 0.24)

;; ============================================================
;; STRUCTS
;; ============================================================
(struct_item
  name: (type_identifier) @rust.struct.name
  (#set! "kind" "RustStruct"))

;; ============================================================
;; ENUMS
;; ============================================================
(enum_item
  name: (type_identifier) @rust.enum.name
  (#set! "kind" "RustEnum"))

;; ============================================================
;; TRAITS
;; ============================================================
(trait_item
  name: (type_identifier) @rust.trait.name
  (#set! "kind" "RustTrait"))

;; ============================================================
;; IMPL BLOCKS (trait implementations and inherent impls)
;; Note: impl blocks are NOT stored as entities in Neo4j.
;; Trait implementations are captured as IMPLEMENTS relationships
;; via collect_rust_trait_implementations() in rust.rs
;; ============================================================

;; ============================================================
;; FUNCTIONS (Top-level) and METHODS (inside impl blocks)
;; Note: All function_item nodes are captured here.
;; Post-processing in rust.rs will reclassify functions inside
;; impl blocks as RustMethod entities.
;; ============================================================
(function_item
  name: (identifier) @rust.function.name
  (#set! "kind" "RustFunction"))

;; ============================================================
;; MODULES
;; ============================================================
(mod_item
  name: (identifier) @rust.module.name
  (#set! "kind" "RustModule"))

;; ============================================================
;; UNIONS
;; ============================================================
(union_item
  name: (type_identifier) @rust.union.name
  (#set! "kind" "RustUnion"))

