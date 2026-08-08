// VARNISH_SPHINX_TOKEN_INCLUDE_01 — included backends
backend included_backend_one {
    .host = "192.168.1.1";
    .port = "80";
    .probe = included_probe;
}

// VARNISH_SPHINX_TOKEN_PROBE_INCLUDE_01
probe included_probe {
    .url = "/api/health";
    .expected_response = 200;
}

// VARNISH_SPHINX_TOKEN_BACKEND_INCLUDE_02
backend included_backend_two {
    .host = "192.168.1.2";
    .port = "80";
}
