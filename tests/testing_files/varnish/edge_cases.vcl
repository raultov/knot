// VARNISH_SPHINX_TOKEN_EDGE_01 — Gotcha 1: ACL mask outside quotes
acl mask_test {
    "192.0.2.0"/24;
    "firewall.example.com" / 24;
}

// VARNISH_SPHINX_TOKEN_EDGE_02 — Gotcha 2: Adjacent string concat
probe concat_probe {
    .request =
        "GET /healthz HTTP/1.1"
        "Host: example.com"
        "Connection: close";
}

// VARNISH_SPHINX_TOKEN_EDGE_03 — Gotcha 3: Attribute names with dots
backend dot_test {
    .host = "127.0.0.1";
    .port = "8080";
}

// VARNISH_SPHINX_TOKEN_EDGE_05 — Gotcha 5: Hyphens in identifiers
sub test_hyphens {
    set req.http.X-Forwarded-For = "proxy";
    set req.http.X-Custom-Header = "value";
    if (req.http.Some-Long-Header-Name ~ "test") {
        return (pass);
    }
}

// VARNISH_SPHINX_TOKEN_EDGE_06 — Gotcha 6: Duration maximal-munch
backend dur_test {
    .host = "127.0.0.1";
    .connect_timeout = 10ms;
    .first_byte_timeout = 10m;
    .between_bytes_timeout = 1.5s;
}

// VARNISH_SPHINX_TOKEN_EDGE_07 — Gotcha 7: Quoted header name
sub quoted_header {
    set req.http."grammatically.valid" = "1";
    set req.http."0-header" = "value";
}

// VARNISH_SPHINX_TOKEN_EDGE_08 — Gotcha 8: Long strings
sub long_strings {
    set req.http.long = {"long string
spanning lines and containing " double quotes"};
    set req.http.triple = """triple-quoted
long string""";
}

// VARNISH_SPHINX_TOKEN_EDGE_09 — Gotcha 9: Extended status code
sub extended_status {
    return (synth(12404));
}

// VARNISH_SPHINX_TOKEN_EDGE_10 — Gotcha 10: Comment styles
# shell comment
sub comment_test {
    // c++ comment
    /* block comment
       multi-line */
    return (pass);
}

// VARNISH_SPHINX_TOKEN_EDGE_15 — Gotcha 15: No escape processing
sub no_escape {
    set req.url = regsub(req.url, "\?$", "");
    set req.url = regsub(req.url, "\R$", "");
}
