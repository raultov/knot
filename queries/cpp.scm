; queries/cpp.scm
(namespace_definition name: (namespace_identifier) @cpp_namespace.name)
(class_specifier name: (type_identifier) @cpp_class.name)
(struct_specifier name: (type_identifier) @c_struct.name)

; --- Function definitions: direct function_declarator ---
(function_definition 
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(function_definition 
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(function_definition 
  declarator: (function_declarator declarator: (operator_name) @cpp_method.name)
)

; --- Function definitions: reference return type (String&) ---
(function_definition 
  declarator: (reference_declarator (function_declarator declarator: (identifier) @cpp_method.name))
)
(function_definition 
  declarator: (reference_declarator (function_declarator declarator: (field_identifier) @cpp_method.name))
)
(function_definition 
  declarator: (reference_declarator (function_declarator declarator: (operator_name) @cpp_method.name))
)

; --- Function definitions: pointer return type (String*) ---
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (identifier) @cpp_method.name))
)
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (field_identifier) @cpp_method.name))
)
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (operator_name) @cpp_method.name))
)

; --- Function definitions: scoped (ClassName::method) ---
(function_definition 
  declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name))
)
(function_definition 
  declarator: (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name))
)
(function_definition 
  declarator: (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name))
)

; --- Function definitions: reference return + scoped (String & String::method) ---
(function_definition 
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name)))
)
(function_definition 
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name)))
)
(function_definition 
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name)))
)

; --- Function definitions: pointer return + scoped (String * String::method) ---
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name)))
)
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name)))
)
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name)))
)

; --- Declarations (no body): direct ---
(declaration
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(declaration
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(declaration
  declarator: (function_declarator declarator: (operator_name) @cpp_method.name)
)

; --- Declarations (no body): reference return ---
(declaration
  declarator: (reference_declarator (function_declarator declarator: (identifier) @cpp_method.name))
)
(declaration
  declarator: (reference_declarator (function_declarator declarator: (field_identifier) @cpp_method.name))
)
(declaration
  declarator: (reference_declarator (function_declarator declarator: (operator_name) @cpp_method.name))
)

; --- Declarations (no body): pointer return ---
(declaration
  declarator: (pointer_declarator (function_declarator declarator: (identifier) @cpp_method.name))
)
(declaration
  declarator: (pointer_declarator (function_declarator declarator: (field_identifier) @cpp_method.name))
)
(declaration
  declarator: (pointer_declarator (function_declarator declarator: (operator_name) @cpp_method.name))
)

; --- Declarations (no body): scoped ---
(declaration
  declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name))
)
(declaration
  declarator: (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name))
)
(declaration
  declarator: (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name))
)

; --- Declarations (no body): reference return + scoped ---
(declaration
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name)))
)
(declaration
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name)))
)
(declaration
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name)))
)

; --- Declarations (no body): pointer return + scoped ---
(declaration
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name)))
)
(declaration
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name)))
)
(declaration
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name)))
)

; --- Field declarations (methods in class body with no body) ---
(field_declaration
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(field_declaration
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(field_declaration
  declarator: (function_declarator declarator: (operator_name) @cpp_method.name)
)

; --- Field declarations (methods in class body with no body): scoped ---
(field_declaration
  declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name))
)
(field_declaration
  declarator: (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name))
)
(field_declaration
  declarator: (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name))
)

; --- Field declarations: reference return + scoped ---
(field_declaration
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name)))
)
(field_declaration
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name)))
)
(field_declaration
  declarator: (reference_declarator (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name)))
)

; --- Field declarations: pointer return + scoped ---
(field_declaration
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name)))
)
(field_declaration
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (field_identifier) @cpp_method.name)))
)
(field_declaration
  declarator: (pointer_declarator (function_declarator declarator: (qualified_identifier name: (operator_name) @cpp_method.name)))
)

(preproc_include path: (_) @preproc.include)
(preproc_def name: (identifier) @preproc.macro)
