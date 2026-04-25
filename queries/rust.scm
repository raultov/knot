;; Tree-sitter query file for Rust entity extraction
;; Version: 0.4 (adds type_alias, constant, static, macro_def, macro_invoke)

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

;; ============================================================
;; TYPE ALIASES
;; ============================================================
(type_item
  name: (type_identifier) @rust.type_alias.name
  type: (_) @signature
  (#set! "kind" "RustTypeAlias"))

;; ============================================================
;; CONSTANTS
;; ============================================================
(const_item
  name: (identifier) @rust.constant.name
  type: (_) @signature
  (#set! "kind" "RustConstant"))

;; ============================================================
;; STATICS
;; ============================================================
(static_item
  name: (identifier) @rust.static.name
  type: (_) @signature
  (#set! "kind" "RustStatic"))

;; ============================================================
;; MACRO DEFINITIONS
;; ============================================================
(macro_definition
  name: (identifier) @rust.macro_def.name
  (#set! "kind" "RustMacroDef"))

;; ============================================================
;; MACRO INVOCATIONS
;; ============================================================
(macro_invocation
  macro: [
    ((identifier) @rust.macro_invoke.name
      (#set! "kind" "RustMacroInvoke"))
    (scoped_identifier
      name: (identifier) @rust.macro_invoke.name
      (#set! "kind" "RustMacroInvoke"))
  ])

