; SCSS Query Language (SQF) for Tree-sitter
; Extracts mixins, functions, variables, and selectors

; Mixins
(mixin_statement
  name: (identifier) @scss.mixin)

; Functions
(function_statement
  name: (identifier) @scss.function)

; Variables ($variable-name)
(variable) @scss.variable

; Class selectors (.my-class)
(class_selector
  (class_name) @css.class)

; ID selectors (#my-id)
(id_selector
  (id_name) @css.id)

; CSS Custom Properties (variables: --my-var)
(declaration
  (property_name) @css.variable
  (#match? @css.variable "^--"))
