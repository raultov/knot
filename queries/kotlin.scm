; ============================================================
; Built-in Tree-sitter extraction queries for Kotlin.
; Compatible with tree-sitter-kotlin-ng v1.1.0
;
; Capture names drive the parser in src/pipeline/parser/languages/kotlin.rs:
;   @kotlin_class.name           — name node of a class declaration
;   @kotlin_interface.name       — name node of an interface declaration
;   @kotlin_object.name          — name node of an object declaration
;   @kotlin_companion.name       — name node of a companion object
;   @kotlin_function.name        — name node of a function declaration
;   @kotlin_method.name          — name node of a method declaration
;   @kotlin_property.name        — name node of a property declaration
;   @signature                   — parameter list node
;   @doc                         — comment node immediately preceding the declaration
;
; Note: Extension functions, type references, and annotations are extracted via
; direct AST traversal in src/pipeline/parser/languages/kotlin.rs for better reliability.
;
; Override this file by placing a custom kotlin.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- Class declarations ---
; In tree-sitter-kotlin-ng, classes use class_declaration with 'class' keyword
(class_declaration
  name: (identifier) @kotlin_class.name)

; --- Interface declarations ---
; In tree-sitter-kotlin-ng, interfaces are also class_declaration but with 'interface' keyword
; We cannot distinguish them in the query, so we capture all class_declaration
; The Rust code will need to check the source text to differentiate

; --- Object declarations (singletons) ---
(object_declaration
  name: (identifier) @kotlin_object.name)

; --- Companion object declarations ---
(companion_object
  name: (identifier)? @kotlin_companion.name)

; --- Function declarations ---
; Captures both top-level functions and methods (distinction made in Rust code)
(function_declaration
  name: (identifier) @kotlin_function.name
  (function_value_parameters) @signature)

; --- Property declarations (val/var) ---
; Properties can have variable_declaration or multi_variable_declaration
(property_declaration
  (variable_declaration
    (identifier) @kotlin_property.name))

; --- Function calls ---
; Simple function call: functionName()
(call_expression
  (identifier) @call.method)

; Navigation calls: object.method()
; The Kotlin parser will extract these via AST traversal
