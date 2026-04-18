; ============================================================
; Built-in Tree-sitter extraction queries for Kotlin.
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
(class_declaration
  name: (simple_identifier) @kotlin_class.name)

; --- Interface declarations ---
(interface_declaration
  name: (simple_identifier) @kotlin_interface.name)

; --- Object declarations (singletons) ---
(object_declaration
  name: (simple_identifier) @kotlin_object.name)

; --- Companion object declarations ---
(companion_object
  name: (simple_identifier)? @kotlin_companion.name)

; --- Top-level function declarations ---
(function_declaration
  name: (simple_identifier) @kotlin_function.name
  parameters: (function_value_parameters) @signature)

; --- Method declarations inside classes ---
(function_declaration
  name: (simple_identifier) @kotlin_method.name
  parameters: (function_value_parameters) @signature)

; --- Property declarations (val/var) ---
(property_declaration
  name: (simple_identifier) @kotlin_property.name)

; --- Method and function invocations ---
; Direct function calls: method()
(call_expression
  (navigation_suffix
    (simple_identifier) @call.method))

; Method calls with receiver: object.method()
(call_expression
  (postfix_expression
    (navigation_suffix
      (simple_identifier) @call.receiver
      "."
      (simple_identifier) @call.method)))

; --- Object creation (Instantiation) ---
; Matches patterns like: MyClass()
(call_expression
  (simple_identifier) @call.method)

; --- Type references in constructor parameters, field types, etc. ---
; These are handled via AST traversal in src/pipeline/parser/languages/kotlin.rs
