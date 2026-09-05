use crate::models::ReferenceIntent;

/// Handle DOM and CSS reference captures in JavaScript
/// (dom.element_id, css.class_name, etc.)
pub(crate) fn handle_dom_css_capture(
    cap_name: &str,
    text: &str,
    line: usize,
) -> Option<ReferenceIntent> {
    match cap_name {
        "dom.element_id" => {
            let clean_id = text
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'')
                .to_string();

            Some(ReferenceIntent::DomElementReference {
                element_id: clean_id,
                line,
            })
        }
        "css.class_name" | "css.class_assignment" => {
            let clean_class = text
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'')
                .to_string();

            Some(ReferenceIntent::CssClassUsage {
                class_name: clean_class,
                line,
            })
        }
        _ => None,
    }
}
