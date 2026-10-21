package com.example.knot.ioverload.api;

import java.util.List;

/**
 * Same-named overloads with different arity (issue #30): implementations of
 * the two-parameter method must not be linked to the one-parameter default.
 */
public interface SettingsService {
    default void applySettings(List<Setting> items) {
        applySettings(items, false);
    }

    void applySettings(List<Setting> items, boolean skipValidation);
}
