// VARNISH_SPHINX_TOKEN_42 — unique token for E2E assertions
vcl 4.1;

import std;
import directors;
import std as standard;

include "backends.vcl";

// VARNISH_SPHINX_TOKEN_BACKEND_01
backend backend_default {
    .host = "127.0.0.1";
    .port = "8080";
    .host_header = "example.com";
    .connect_timeout = 1s;
    .first_byte_timeout = 60s;
    .between_bytes_timeout = 60s;
    .max_connections = 100;
    .probe = probe_health;
}

// VARNISH_SPHINX_TOKEN_PROBE_01
probe probe_health {
    .url = "/healthz";
    .expected_response = 200;
    .timeout = 2s;
    .interval = 5s;
    .window = 8;
    .threshold = 3;
}

// VARNISH_SPHINX_TOKEN_ACL_01
acl acl_localnetwork {
    "localhost";
    "192.0.2.0"/24;
    "10.0.0.0"/8;
    ! "192.0.2.23";
}

// VARNISH_SPHINX_TOKEN_SUB_01
sub pipe_if_local {
    if (client.ip ~ acl_localnetwork) {
        return (pipe);
    }
}

// VARNISH_SPHINX_TOKEN_BUILTIN_SUB_01
sub vcl_recv {
    call pipe_if_local;

    if (req.url == "/healthz") {
        return (synth(200));
    }

    if (req.http.host ~ "^(www\.)?example\.com$") {
        set req.backend_hint = backend_default;
    }

    if (client.ip ~ acl_localnetwork) {
        std.log("Request from local network");
    }

    return (hash);
}

// VARNISH_SPHINX_TOKEN_BUILTIN_SUB_02
sub vcl_deliver {
    if (obj.hits > 0) {
        set resp.http.X-Cache = "HIT";
    } else {
        set resp.http.X-Cache = "MISS";
    }
    standard.log("served response");
    return (deliver);
}

// VARNISH_SPHINX_TOKEN_INIT_01
sub vcl_init {
    new cluster_director = directors.round_robin();
    cluster_director.add_backend(backend_default);
    return (ok);
}

// VARNISH_SPHINX_TOKEN_UNUSED_01 unused b1
backend b1 { .host = "10.0.0.1"; }

unused b1;
backend default none;
