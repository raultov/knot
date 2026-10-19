package com.example.knot.ifields.web;

import com.example.knot.ifields.api.PreferenceService;
import java.util.List;

/**
 * Field name does not match the interface name: the call edge is dropped (issue #28).
 */
public class RenamedFieldPreferenceController {
    private final PreferenceService prefs;

    public RenamedFieldPreferenceController(PreferenceService prefs) {
        this.prefs = prefs;
    }

    public void storeViaRenamedField(List<String> items) {
        prefs.store(items);
    }
}
