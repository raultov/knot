; ============================================================
; TSX-specific Tree-sitter extraction queries for React components.
;
; This file contains rules that are ONLY valid when parsing TSX
; (React component syntax). It is concatenated with the base
; TypeScript rules when processing .tsx files.
;
; Override this file by placing a custom tsx.scm in the directory
; pointed to by --custom-queries-path / KNOT_CUSTOM_QUERIES_PATH.
; ============================================================

; --- JSX Component Invocation (React/TSX) ---
; Captures component invocations via JSX syntax
; Matches: <ChartToolbar />
(jsx_self_closing_element
  name: (_) @call.method)

; Matches: <ChartToolbar>...</ChartToolbar>
(jsx_opening_element
  name: (_) @call.method)
