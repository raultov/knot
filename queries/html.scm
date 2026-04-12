; ============================================================
; HTML Tree-sitter extraction queries for Angular & Web Components
;
; Captures:
; - Custom elements (Web Components, Angular components)
; - id attributes
; - class attributes
;
; Standard HTML5 elements (div, span, p) are ignored to avoid
; saturating the knowledge graph with low-value entities.
; ============================================================

; Web Components / Custom Elements
; Matches: <app-user-profile>, <web-component>, <sheet-dialog>
; Ignores standard HTML tags (no hyphens)
((element
   (start_tag
     (tag_name) @html_element_name))
 (#match? @html_element_name "-"))

; id attributes on any element
(attribute
  (attribute_name) @attr_name
  (quoted_attribute_value) @html_id_value
  (#eq? @attr_name "id"))

; class attributes on any element
(attribute
  (attribute_name) @attr_name
  (quoted_attribute_value) @html_class_value
  (#eq? @attr_name "class"))

; Comments (for documentation)
(comment) @doc
