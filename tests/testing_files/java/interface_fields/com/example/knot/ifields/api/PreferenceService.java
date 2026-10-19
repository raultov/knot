package com.example.knot.ifields.api;

import java.util.List;

/**
 * Interface with an overloaded default method (issue #28, second variant).
 */
public interface PreferenceService {
    default void store(List<String> items) {
        store(items, false);
    }

    void store(List<String> items, boolean skipValidation);
}
