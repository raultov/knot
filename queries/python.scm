; Tree-sitter query file for Python entity extraction (Phase 2)
; Supports: PythonClass, PythonFunction, PythonMethod
;
; IMPORTANT: Python has a single `function_definition` node type for both
; top-level functions and class methods. The distinction is made in
; `handle_python_capture` by inspecting the parent node's context.
;
; Capture naming convention:
;   @python.class.name    — class definition name
;   @python.function.name — any function definition (post-processing detects if it's a method)
;   @python.signature     — parameters (optional)

; ============================================================
; CLASSES
; ============================================================
(class_definition
  name: (identifier) @python.class.name)

; ============================================================
; FUNCTIONS (top-level and methods — same node type in Python)
; Post-processing in handle_python_capture distinguishes them
; by checking if the function_definition is inside a class body.
; ============================================================

; Function with explicit parameters capture
(function_definition
  name: (identifier) @python.function.name
  parameters: (parameters) @python.signature)

; Function without parameters (fallback for alternate parses)
(function_definition
  name: (identifier) @python.function.name)

; ============================================================
; ASYNC FUNCTIONS (also function_definition, same handling)
; Note: tree-sitter-python uses optional 'async' modifier on function_definition,
; not a separate async_function_definition node type.
; ============================================================
; Async functions are already captured by the function_definition rules above
; (the optional 'async' modifier is part of the function_definition structure)

; ============================================================
; LAMBDA FUNCTIONS (stored in assignment: f = lambda x: ...)
; Captured as PythonFunction since they are effectively assignments
; ============================================================
(assignment
  left: (identifier) @python.function.name
  right: (lambda))

; ============================================================
; CONSTANTS (Phase 4)
; UPPER_CASE identifiers that are simple assignments at module level
; ============================================================
(assignment
  left: (identifier) @python.constant.name
  right: (_)
  (#match? @python.constant.name "^[A-Z][A-Z0-9_]*$"))
