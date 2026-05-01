; queries/cpp.scm
(namespace_definition name: (namespace_identifier) @cpp_namespace.name)
(class_specifier name: (type_identifier) @cpp_class.name)
(struct_specifier name: (type_identifier) @c_struct.name)
(function_definition 
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(function_definition 
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(declaration
  type: (primitive_type)
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(declaration
  type: (primitive_type)
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(declaration
  type: (type_identifier)
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(declaration
  type: (type_identifier)
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(field_declaration
  type: (_)
  declarator: (function_declarator declarator: (field_identifier) @cpp_method.name)
)
(field_declaration
  type: (_)
  declarator: (function_declarator declarator: (identifier) @cpp_method.name)
)
(function_definition 
  declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @cpp_method.name))
)

(preproc_include path: (_) @preproc.include)
(preproc_def name: (identifier) @preproc.macro)
