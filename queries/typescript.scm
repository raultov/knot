; ============================================================
; Built-in Tree-sitter extraction queries for TypeScript / TSX.
;
; Capture names drive the parser in src/pipeline/parse.rs:
;   @class.name      — name node of a class declaration
;   @interface.name  — name node of an interface declaration
;   @method.name     — name node of a method definition
;   @function.name   — name node of a function declaration
;   @constant.name   — name node of a top-level const or static readonly property
;   @enum.name       — name node of an enum declaration
;   @signature       — parameter list node (type params included)
;   @doc             — comment node immediately preceding the declaration
;   @class.extends   — parent class in extends clause
;   @class.implements — interface in implements clause
;   @type.reference  — type reference in annotations (variable declarations, method signatures)
;
; Override this file by placing a custom typescript.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- Class declarations ---
; Comments are handled dynamically in Rust.
(class_declaration
  name: (type_identifier) @class.name)

(abstract_class_declaration
  name: (type_identifier) @class.name)

; --- Class inheritance (extends clause) ---
; NOTE: Tree-sitter's TypeScript grammar does not expose extends/implements as named fields
; They are captured as part of the class_declaration node, but we need to handle them
; differently via AST traversal in the parser. This is handled in src/pipeline/parse.rs
; by examining child nodes of class declarations directly.
; (class_declaration
;   superclass: (type_identifier) @class.extends)

; --- Class interface implementation (implements clause) ---
; NOTE: Similar to extends, implements are not exposed as named fields
; They will be extracted via AST traversal in the parser
; (class_declaration
;   implements: (implements_clause
;     (type_identifier) @class.implements))

; --- Interface declarations ---
(interface_declaration
  name: (type_identifier) @interface.name)

; --- Method definitions (inside class bodies) ---
(method_definition
  name: (property_identifier) @method.name
  parameters: (formal_parameters) @signature)

(method_signature
  name: (property_identifier) @method.name
  parameters: (formal_parameters) @signature)

(abstract_method_signature
  name: (property_identifier) @method.name
  parameters: (formal_parameters) @signature)

; --- Top-level function declarations ---
(function_declaration
  name: (identifier) @function.name
  parameters: (formal_parameters) @signature)

; --- Arrow function assigned to a variable (const foo = () => ...) ---
(lexical_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function
      parameters: (formal_parameters) @signature)))

; --- Top-level const declarations (const MY_CONST = ...) ---
; These become Constant entities
(lexical_declaration
  (variable_declarator
    name: (identifier) @constant.name))

; --- Default export arrow function (export default () => {...}) ---
; We'll capture the "default" keyword as a synthetic name for the function
(export_statement
  "default" @function.name
  (arrow_function
    parameters: (formal_parameters) @signature))

; --- Enum declarations ---
(enum_declaration
  name: (identifier) @enum.name)

; --- Static readonly class properties (static readonly CONST_VAL = ...) ---
(public_field_definition
  name: (property_identifier) @constant.name)

; --- Method and function invocations (call expressions) ---
; Matches: this.method(), object.method(), Class.method(), localCall()
;
; Pattern 1: Calls with a receiver via member_expression (e.g., this.foo(), obj.bar(), Class.baz())
(call_expression
  function: (member_expression
    object: (this) @call.receiver
    "."
    property: (property_identifier) @call.method))

(call_expression
  function: (member_expression
    object: (identifier) @call.receiver
    "."
    property: (property_identifier) @call.method))

; Pattern 2: Direct function/method calls without receiver (e.g., localCall())
(call_expression
  function: (identifier) @call.method)

; --- Object creation (Instantiation) ---
; Matches patterns like: new MyClass()
(new_expression
  constructor: (identifier) @call.method)

; --- Type references in variable declarations ---
; Matches: const prompt: DeviceRequestPrompt = ...
; or: private prop: SomeType;
(variable_declarator
  type: (type_annotation
    (type_identifier) @type.reference))

(property_signature
  type: (type_annotation
    (type_identifier) @type.reference))

(public_field_definition
  type: (type_annotation
    (type_identifier) @type.reference))

; --- Type references in function/method return types ---
; Matches: function foo(): ReturnType { ... }
; or: method(): PromiseType { ... }
(function_declaration
  return_type: (type_annotation
    (type_identifier) @type.reference))

(method_definition
  return_type: (type_annotation
    (type_identifier) @type.reference))

(method_signature
  return_type: (type_annotation
    (type_identifier) @type.reference))

(arrow_function
  return_type: (type_annotation
    (type_identifier) @type.reference))

; --- Type references in formal parameters ---
; NOTE: Tree-sitter's TypeScript grammar doesn't expose parameter types as named fields
; in a way that's easily queryable. These are extracted via other mechanisms.
; Removed: problematic query for parameter type annotations

