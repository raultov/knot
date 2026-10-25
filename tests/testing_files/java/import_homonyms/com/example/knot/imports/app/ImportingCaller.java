package com.example.knot.imports.app;

import com.example.knot.imports.b.Config;

public class ImportingCaller {
    public String loadImported(Config config) {
        return config.load();
    }

    public void resetImported(Config config) {
        config.reset();
    }
}
