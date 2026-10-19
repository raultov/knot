package com.example.knot.ifields.web;

import com.example.knot.ifields.api.PreferenceService;
import java.util.List;

/**
 * Field name is the decapitalized interface name: resolves today (guard).
 */
public class MatchingFieldPreferenceController {
    private final PreferenceService preferenceService;

    public MatchingFieldPreferenceController(PreferenceService preferenceService) {
        this.preferenceService = preferenceService;
    }

    public void storeViaMatchingField(List<String> items) {
        this.preferenceService.store(items);
    }
}
