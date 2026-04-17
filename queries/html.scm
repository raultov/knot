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

; ============================================================
; Phase 4: HTML-to-JS and HTML-to-CSS Linking
; ============================================================

; Script imports via <script src="...">
(element
  (start_tag
    (tag_name) @script_tag
    (attribute
      (attribute_name) @src_attr
      (quoted_attribute_value) @script_src))
  (#eq? @script_tag "script")
  (#eq? @src_attr "src"))

; Stylesheet imports via <link rel="stylesheet" href="...">
(element
  (start_tag
    (tag_name) @link_tag
    (attribute
      (attribute_name) @rel_attr
      (quoted_attribute_value) @rel_value)
    (attribute
      (attribute_name) @href_attr
      (quoted_attribute_value) @stylesheet_href))
  (#eq? @link_tag "link")
  (#eq? @rel_attr "rel")
  (#eq? @rel_value "\"stylesheet\"" "stylesheet" "'stylesheet'")
  (#eq? @href_attr "href"))

; Alternative: stylesheet link with any order of attributes
(element
  (start_tag
    (tag_name) @link_tag
    (attribute
      (attribute_name) @href_attr
      (quoted_attribute_value) @stylesheet_href))
  (#eq? @link_tag "link")
  (#eq? @href_attr "href"))
