// VARNISH_SPHINX_TOKEN_INLINE_PROBE_01 — .probe = { } anonymous form
backend b_inline {
    .host = "127.0.0.1";
    .probe = {
        .url = "/healthz";
        .timeout = 1s;
        .interval = 5s;
        .window = 5;
        .threshold = 3;
    };
}

// VARNISH_SPHINX_TOKEN_INLINE_PROBE_02 — .probe = named reference
probe named_health {
    .url = "/health";
    .expected_response = 200;
}

backend b_named {
    .host = "127.0.0.1";
    .probe = named_health;
}
