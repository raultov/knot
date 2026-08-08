// VARNISH_SPHINX_TOKEN_MULTI_B_01 — sub vcl_recv part 2
sub vcl_recv {
    if (req.url == "/part2") {
        return (synth(702));
    }
    // VARNISH_SPHINX_TOKEN_MULTI_B_BODY — searchable body
    std.log("vcl_recv part 2 executed");
}
