package com.example.knot.imports.b;

public class SamePackageCaller {
    public String loadSame(Config config) {
        return config.load();
    }
}
