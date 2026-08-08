// This is a Fastly VCL file that must yield NO entities.
vcl 4.0;

// Fastly marker: declare local
declare local var.x STRING;

// Fastly marker: table definition
table my_table {
    "key": "value",
}

// Fastly marker: director keyword
director my_dir random {
    .retries = 5;
}

sub vcl_fetch {
    set beresp.ttl = 60s;
}

sub vcl_log {
    std.log("Fastly log");
}

sub vcl_recv {
    error 750 "test error";
}
