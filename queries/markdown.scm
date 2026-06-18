; Tree-sitter query file for Markdown entity extraction.
; Supports: MarkdownDocument, MarkdownSection
;
; tree-sitter-md emits sections hierarchically: the `section` node spans
; from an ATX heading down to the next same-or-higher-level heading. This
; means capturing the `section` node directly gives us the right span for
; both `end_line` and the embedded body text — no parent-walking or
; level-tracking needed in the handler.
;
; Capture naming convention:
;   @markdown.document.name — the document root, one per file
;   @markdown.section       — every ATX heading at any depth produces one
;                             section entity covering its heading + body
;
; ============================================================
; DOCUMENTS
; ============================================================
(document) @markdown.document.name

; ============================================================
; SECTIONS (one per ATX heading, hierarchically nested)
; The section node spans heading + body. The handler in
; languages/markdown.rs reads the heading text from the section's
; first atx_heading child to derive the entity name.
; ============================================================
(section) @markdown.section
