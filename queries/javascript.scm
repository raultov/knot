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

; --- Arrow functions and function expressions assigned to variables (const/let) ---
; Matches: const foo = () => {}; let bar = function() {};
; Uses lexical_declaration for const/let declarations
(lexical_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function
      parameters: (formal_parameters) @signature)))

(lexical_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (function_expression
      parameters: (formal_parameters) @signature)))

; --- Arrow functions and function expressions assigned to variables (var) ---
; Matches: var foo = () => {}; var bar = function() {};
; Uses variable_declaration for var declarations
(variable_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function
      parameters: (formal_parameters) @signature)))

(variable_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (function_expression
      parameters: (formal_parameters) @signature)))

; --- Top-level constant declarations (non-function values, const/let) ---
; Matches: const FOO = 42; let CONFIG = {}; const NAME = "value";
; IMPORTANT: Explicitly lists value types to exclude arrow_function/function_expression
; This prevents duplication with the function patterns above
; Uses lexical_declaration for const/let
(lexical_declaration
  (variable_declarator
    name: (identifier) @constant.name
    value: [
      (number)
      (string)
      (template_string)
      (true)
      (false)
      (null)
      (undefined)
      (object)
      (array)
      (identifier)
      (member_expression)
      (call_expression)
      (await_expression)
      (binary_expression)
      (unary_expression)
      (ternary_expression)
      (new_expression)
      (class)
      (regex)
    ]))

; --- Top-level constant declarations (non-function values, var) ---
; Matches: var FOO = 42; var CONFIG = {}; var NAME = "value";
; IMPORTANT: Explicitly lists value types to exclude arrow_function/function_expression
; Uses variable_declaration for var
(variable_declaration
  (variable_declarator
    name: (identifier) @constant.name
    value: [
      (number)
      (string)
      (template_string)
      (true)
      (false)
      (null)
      (undefined)
      (object)
      (array)
      (identifier)
      (member_expression)
      (call_expression)
      (await_expression)
      (binary_expression)
      (unary_expression)
      (ternary_expression)
      (new_expression)
      (class)
      (regex)
    ]))

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
; (field_definition
;   property: (property_identifier) @constant.name)

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

; ============================================================
; Phase 4: Cross-Language References (DOM and CSS)
; ============================================================

; --- DOM Element References ---
; Matches: document.getElementById('element-id')
; Captures both the method name and the string argument
(call_expression
  function: (member_expression
    object: (member_expression
      object: (identifier) @dom.receiver
      property: (property_identifier) @dom.method)
    property: (property_identifier) @dom.action)
  arguments: (arguments
    (string) @dom.element_id))

; Shorter form: element.getElementById() without nested member_expression
(call_expression
  function: (member_expression
    object: (identifier) @dom.receiver
    property: (property_identifier) @dom.action)
  arguments: (arguments
    (string) @dom.element_id))

; --- CSS Class Manipulations ---
; Matches: element.classList.add('class-name')
(call_expression
  function: (member_expression
    object: (member_expression
      object: (identifier) @css.receiver
      property: (property_identifier) @css.classList)
    property: (property_identifier) @css.method)
  arguments: (arguments
    (string) @css.class_name))

; --- CSS Class Assignments ---
; Matches: element.className = 'new-class' or element.className = "new-class"
(assignment_expression
  left: (member_expression
    object: (identifier) @css.receiver
    property: (property_identifier) @css.className)
  right: (string) @css.class_assignment)
