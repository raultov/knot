; ============================================================
; Built-in Tree-sitter extraction queries for JavaScript / Node.js.
;
; Capture names drive the parser in src/pipeline/parser.rs:
;   @class.name      — name node of a class declaration
;   @method.name     — name node of a method definition
;   @function.name   — name node of a function declaration
;   @constant.name   — name node of a top-level const or static readonly property
;   @signature       — parameter list node
;   @doc             — comment node immediately preceding the declaration
;   @class.extends   — parent class in extends clause
;
; Override this file by placing a custom javascript.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- Class declarations ---
(class_declaration
  name: (identifier) @class.name)

; --- Class inheritance (extends clause) ---
; In JavaScript, the class_heritage node contains the parent class directly
; This is handled dynamically in Rust for now
(class_declaration
  (class_heritage
    (identifier) @class.extends))

; --- Method definitions (inside class bodies) ---
(method_definition
  name: (property_identifier) @method.name
  parameters: (formal_parameters) @signature)

; --- Top-level function declarations ---
(function_declaration
  name: (identifier) @function.name
  parameters: (formal_parameters) @signature)

; --- Arrow function assigned to a variable (const foo = () => ...) ---
(variable_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function
      parameters: (formal_parameters) @signature)))

; Alternative pattern for const/let/var with arrow functions
(variable_declarator
  name: (identifier) @function.name
  value: (arrow_function
    parameters: (formal_parameters) @signature))

; --- Top-level const/let/var declarations (const MY_CONST = ...) ---
; These become Constant entities
(variable_declaration
  (variable_declarator
    name: (identifier) @constant.name))

(variable_declarator
  name: (identifier) @constant.name)

; --- Default export function declaration (export default function() {...}) ---
(export_statement
  (function_declaration
    name: (identifier) @function.name
    parameters: (formal_parameters) @signature))

; --- Default export arrow function (export default () => {...}) ---
(export_statement
  (arrow_function
    parameters: (formal_parameters) @signature))

; --- Named export function declaration (export function foo() {...}) ---
(export_statement
  (function_declaration
    name: (identifier) @function.name
    parameters: (formal_parameters) @signature))

; --- Static class properties and methods ---
(public_field_definition
  name: (property_identifier) @constant.name)

; --- Method and function invocations (call expressions) ---
; Matches: this.method(), object.method(), Class.method(), localCall()
;
; Pattern 1: Calls with a receiver via member_expression (e.g., this.foo(), obj.bar(), Class.baz())
(call_expression
  function: (member_expression
    object: (this) @call.receiver
    property: (property_identifier) @call.method))

(call_expression
  function: (member_expression
    object: (identifier) @call.receiver
    property: (property_identifier) @call.method))

; Pattern 2: Direct function/method calls without receiver (e.g., localCall())
(call_expression
  function: (identifier) @call.method)

; --- Object creation (Instantiation) ---
; Matches patterns like: new MyClass()
(new_expression
  constructor: (identifier) @call.method)

; --- CommonJS require() calls ---
; Matches: require('module') or require("module")
(call_expression
  function: (identifier) @call.method
  arguments: (arguments
    (string) @call.argument))

; --- Module.exports assignments ---
; Matches: module.exports = ...
(assignment_expression
  left: (member_expression
    object: (identifier) @call.receiver
    property: (property_identifier) @call.method)
  right: (_))
