; queries/c.scm
(struct_specifier name: (type_identifier) @c_struct.name)

; --- Function definitions: direct ---
(function_definition 
  declarator: (function_declarator declarator: (identifier) @c_function.name)
)

; --- Function definitions: pointer return (int*) ---
(function_definition 
  declarator: (pointer_declarator (function_declarator declarator: (identifier) @c_function.name))
)

(preproc_include path: (_) @preproc.include)
(preproc_def name: (identifier) @preproc.macro)
