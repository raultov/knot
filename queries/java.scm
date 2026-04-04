; ============================================================
; Built-in Tree-sitter extraction queries for Java.
;
; Capture names drive the parser in src/pipeline/parse.rs:
;   @class.name      — name node of a class declaration
;   @interface.name  — name node of an interface declaration
;   @method.name     — name node of a method declaration
;   @signature       — full method declarator text (name + params)
;   @doc             — block_comment immediately preceding the node
;
; Override this file by placing a custom java.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- Class declarations ---
; Captures the class name. Comments are handled dynamically in Rust.
(class_declaration
  name: (identifier) @class.name)

; --- Interface declarations ---
(interface_declaration
  name: (identifier) @interface.name)

; --- Method declarations ---
; Captures name and full declarator (signature). Comments are handled dynamically in Rust.
(method_declaration
  name: (identifier) @method.name
  parameters: (formal_parameters) @signature)

; --- Method invocations ---
; Captures method calls with optional receiver (object or class name).
; Matches patterns like: method(), this.method(), object.method(), Class.method()
(method_invocation
  object: (identifier) @call.receiver
  "."
  name: (identifier) @call.method)

(method_invocation
  object: (this) @call.receiver
  "."
  name: (identifier) @call.method)

(method_invocation
  name: (identifier) @call.method)

; --- Object creation (Instantiation) ---
; Matches patterns like: new MyClass()
(object_creation_expression
  type: (type_identifier) @call.method)
