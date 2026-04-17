; CSS Query Language (SQF) for Tree-sitter
; Extracts selectors, variables, and keyframe definitions

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

; Keyframe blocks (@keyframes animation-name)
(at_rule
  (at_keyword) @css.keyframe
  (#match? @css.keyframe "@keyframes"))
