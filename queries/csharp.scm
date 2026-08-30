; ============================================================
; Built-in Tree-sitter extraction queries for C#.
;
; Capture names drive the parser in src/pipeline/parser/mod.rs and are
; routed in src/pipeline/parser/extractor/captures.rs:
;   @csharp.namespace.name   — name node of a namespace declaration (block + file-scoped)
;   @csharp.class.name       — name node of a class declaration
;   @csharp.interface.name   — name node of an interface declaration
;   @csharp.struct.name      — name node of a struct declaration
;   @csharp.record.name      — name node of a record declaration (record class / record struct)
;   @csharp.enum.name        — name node of an enum declaration
;   @csharp.method.name      — name node of a method declaration
;   @csharp.constructor.name — name node of a constructor declaration
;   @csharp.property.name    — name node of a property declaration
;   @csharp.field.name       — variable_declarator name of a field declaration
;   @csharp.delegate.name    — name node of a delegate declaration
;   @csharp.event.name       — name of an event declaration / event field declarator
;   @csharp.indexer          — whole indexer declaration (no name field; name synthesised)
;   @csharp.operator         — whole operator declaration (no name field; name synthesised)
;   @csharp.local_function.name — name node of a local function statement
;   @csharp.signature        — parameter list text (name + params)
;
; Notes (grammar gaps handled in Rust — see docs/specs/csharp_support_plan.md §2.3):
;   - field_declaration has no `name` field; the capture targets the
;     `variable_declarator` identifier and the handler walks up to the
;     declaration. `const` fields become csharp_constant.
;   - record_declaration covers both `record class` and `record struct`;
;     both map to csharp_record.
;   - indexer/operator declarations have no `name` field; names are
;     synthesised (`this[]`, `operator +`).
;
; Override this file by placing a custom csharp.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- Namespace declarations (block + file-scoped forms) ---
(namespace_declaration
  name: (_) @csharp.namespace.name)

(file_scoped_namespace_declaration
  name: (_) @csharp.namespace.name)

; --- Class declarations ---
(class_declaration
  name: (identifier) @csharp.class.name)

; --- Interface declarations ---
(interface_declaration
  name: (identifier) @csharp.interface.name)

; --- Struct declarations ---
(struct_declaration
  name: (identifier) @csharp.struct.name)

; --- Record declarations (record class and record struct) ---
(record_declaration
  name: (identifier) @csharp.record.name)

; --- Enum declarations ---
(enum_declaration
  name: (identifier) @csharp.enum.name)

; --- Method declarations ---
(method_declaration
  name: (identifier) @csharp.method.name
  parameters: (parameter_list) @csharp.signature)

; --- Constructor declarations ---
(constructor_declaration
  name: (identifier) @csharp.constructor.name)

; --- Property declarations ---
(property_declaration
  name: (identifier) @csharp.property.name)

; --- Field declarations ---
; field_declaration has no `name` field (grammar gap): the name lives two
; levels down in variable_declaration > variable_declarator.
(field_declaration
  (variable_declaration
    (variable_declarator
      name: (identifier) @csharp.field.name)))

; --- Delegate declarations ---
(delegate_declaration
  name: (identifier) @csharp.delegate.name
  parameters: (parameter_list) @csharp.signature)

; --- Event declarations (accessor form and event-field form) ---
(event_declaration
  name: (identifier) @csharp.event.name)

(event_field_declaration
  (variable_declaration
    (variable_declarator
      name: (identifier) @csharp.event.name)))

; --- Indexer declarations ---
; No `name` field (grammar gap): capture the whole declaration; the handler
; synthesises the name `this[]`.
(indexer_declaration) @csharp.indexer

; --- Operator declarations ---
; No `name` field (grammar gap): capture the whole declaration; the handler
; synthesises the name `operator <token>`.
(operator_declaration) @csharp.operator

; --- Local function statements ---
(local_function_statement
  name: (identifier) @csharp.local_function.name
  parameters: (parameter_list) @csharp.signature)
