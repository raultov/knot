/// Fastly VCL detection guard.
///
/// Returns `true` if the source contains any Fastly-exclusive marker,
/// meaning the parser must return an empty `Vec` (see spec §1.3).
pub(crate) fn is_fastly_vcl(source: &str) -> bool {
    let s = source;
    s.contains("declare local var.")
        || s.contains("\ntable ")
        || s.starts_with("table ")
        || (s.contains("error ") && !s.contains("vcl_backend_error"))
        || s.contains("sub vcl_fetch")
        || s.contains("sub vcl_log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_fastly_declare_local_var() {
        assert!(is_fastly_vcl(
            r#"vcl 4.0;
declare local var.x STRING;
sub vcl_recv { }"#
        ));
    }

    #[test]
    fn test_is_fastly_table_definition() {
        assert!(is_fastly_vcl(
            r#"vcl 4.0;
table my_table {
    "key": "value",
}"#
        ));
    }

    #[test]
    fn test_is_fastly_sub_vcl_fetch() {
        assert!(is_fastly_vcl(
            r#"vcl 4.0;
sub vcl_fetch {
    set beresp.ttl = 60s;
}"#
        ));
    }

    #[test]
    fn test_is_fastly_sub_vcl_log() {
        assert!(is_fastly_vcl(
            r#"vcl 4.0;
sub vcl_log {
    std.log("test");
}"#
        ));
    }

    #[test]
    fn test_is_fastly_error_statement() {
        assert!(is_fastly_vcl(
            r#"vcl 4.0;
sub vcl_recv {
    error 750 "msg";
}"#
        ));
    }

    #[test]
    fn test_valid_varnish_is_not_fastly() {
        assert!(!is_fastly_vcl(
            r#"vcl 4.1;
import std;
backend default { .host = "127.0.0.1"; }
sub vcl_recv { return (hash); }"#
        ));
    }

    #[test]
    fn test_empty_source_is_not_fastly() {
        assert!(!is_fastly_vcl(""));
    }
}
