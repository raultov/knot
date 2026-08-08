// VARNISH_SPHINX_TOKEN_MULTI_A_01 — sub vcl_recv part 1
sub vcl_recv {
    if (req.url == "/part1") {
        return (synth(701));
    }
    // VARNISH_SPHINX_TOKEN_MULTI_A_BODY — searchable body
    std.log("vcl_recv part 1 executed");
}
