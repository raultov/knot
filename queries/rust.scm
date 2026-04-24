;; Tree-sitter query file for Rust entity extraction
;; Version: 0.1 (based on tree-sitter-rust 0.24)

;; ============================================================
;; STRUCTS
;; ============================================================
(struct_item
  name: (type_identifier) @rust.struct.name
  type_parameters: (type_parameters)? @rust.generics
  (#set! "kind" "RustStruct"))

;; ============================================================
;; ENUMS
;; ============================================================
(enum_item
  name: (type_identifier) @rust.enum.name
  (#set! "kind" "RustEnum"))

;; ============================================================
;; UNIONS
;; ============================================================
(union_item
  name: (type_identifier) @rust.union.name
  (#set! "kind" "RustUnion"))

;; ============================================================
;; TRAITS
;; ============================================================
(trait_item
  name: (type_identifier) @rust.trait.name
  type_parameters: (type_parameters)? @rust.generics
  (#set! "kind" "RustTrait"))

;; ============================================================
;; IMPL BLOCKS (Inherent - no trait)
;; ============================================================
(impl_item
  trait: (NONE)
  type: (generic_type
    type: (type_identifier) @rust.impl.target)
  (#set! "kind" "RustImpl")
  (#set! "impl_trait" "none"))

;; ============================================================
;; IMPL BLOCKS (Trait Implementation)
;; ============================================================
(impl_item
  trait: (generic_type
    type: (type_identifier) @rust.impl.trait)
  type: (generic_type
    type: (type_identifier) @rust.impl.target)
  (#set! "kind" "RustImpl"))

;; ============================================================
;; FUNCTIONS (Top-level)
;; ============================================================
(function_item
  name: (identifier) @rust.function.name
  type_parameters: (type_parameters)? @rust.generics
  parameters: (parameters) @rust.signature
  return_type: (type_annotation)? @rust.return_type
  (#set! "kind" "RustFunction"))

;; ============================================================
;; METHODS (inside impl blocks)
;; ============================================================
(method_declaration
  name: (identifier) @rust.method.name
  type_parameters: (type_parameters)? @rust.generics
  parameters: (parameters) @rust.signature
  return_type: (type_annotation)? @rust.return_type
  (#set! "kind" "RustMethod"))

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
  macro: (identifier) @rust.macro_inv.name
  (#set! "kind" "RustMacroInvoke"))

;; ============================================================
;; TYPE ALIASES
;; ============================================================
(type_alias_item
  name: (type_identifier) @rust.type_alias.name
  type_parameters: (type_parameters)? @rust.generics
  (#set! "kind" "RustTypeAlias"))

;; ============================================================
;; CONSTANTS
;; ============================================================
(const_item
  name: (identifier) @rust.constant.name
  (#set! "kind" "RustConstant"))

;; ============================================================
;; STATICS
;; ============================================================
(static_item
  name: (identifier) @rust.static.name
  (#set! "kind" "RustStatic"))

;; ============================================================
;; MODULES
;; ============================================================
(mod_item
  name: (identifier) @rust.module.name
  (#set! "kind" "RustModule"))

;; ============================================================
;; LIFETIME PARAMETERS
;; ============================================================
(lifetime
  "'" @rust.lifetime)

;; ============================================================
;; ATTRIBUTES (derive, etc.)
;; ============================================================
(attribute_item
  (attribute
    (identifier) @rust.attribute.name
    (token_tree)? @rust.attribute.args))