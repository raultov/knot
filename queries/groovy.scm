; ============================================================
; Built-in Tree-sitter extraction queries for Groovy.
;
; Capture names drive the parser in src/pipeline/parse.rs:
;   @groovy.class.name       — name node of a class declaration
;   @groovy.interface.name   — name node of an interface declaration
;   @groovy.enum.name        — name node of an enum declaration
;   @groovy.method.name      — name node of a method declaration
;   @groovy.field.name       — name node of a field declaration
;   @groovy.closure.name     — name node of a closure (assigned to variable)
;   @groovy.signature        — full method parameters text
;   @doc                     — block_comment immediately preceding the node
;
; Override this file by placing a custom groovy.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- Class declarations ---
(class_declaration
  name: (identifier) @groovy.class.name)

; --- Interface declarations ---
(interface_declaration
  name: (identifier) @groovy.interface.name)

; --- Enum declarations ---
(enum_declaration
  name: (identifier) @groovy.enum.name)

; --- Method declarations ---
(method_declaration
  name: (identifier) @groovy.method.name)

; --- Constructor declarations ---
(constructor_declaration
  name: (identifier) @groovy.method.name)

; --- Function definitions (Groovy `def foo() { }`) ---
(function_definition
  name: (identifier) @groovy.method.name)

(function_definition
  parameters: (formal_parameters) @groovy.signature)

; --- Field declarations (assigned to variable) ---
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @groovy.field.name))

; --- Constant declarations ---
(constant_declaration
  declarator: (variable_declarator
    name: (identifier) @groovy.field.name))

; --- Script-level variable declarations ---
(local_variable_declaration
  declarator: (variable_declarator
    name: (identifier) @groovy.field.name))

; --- Method signatures ---
(method_declaration
  parameters: (formal_parameters) @groovy.signature)

(constructor_declaration
  parameters: (formal_parameters) @groovy.signature)

; --- Method invocations (for call graph) ---
(method_invocation
  name: (identifier) @call.method)

(method_invocation
  object: (identifier) @call.receiver
  "."
  name: (identifier) @call.method)

(method_invocation
  object: (this) @call.receiver
  "."
  name: (identifier) @call.method)

; --- Object creation (instantiation) ---
(object_creation_expression
  type: (type_identifier) @call.method)
