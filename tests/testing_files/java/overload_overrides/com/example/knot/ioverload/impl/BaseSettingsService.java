package com.example.knot.ioverload.impl;

import com.example.knot.ioverload.api.Setting;
import com.example.knot.ioverload.api.SettingsService;
import java.util.List;

/**
 * Declares only the one-parameter overload: subclasses overriding the
 * two-parameter method must walk past it up to `SettingsService`.
 */
public abstract class BaseSettingsService implements SettingsService {
    @Override
    public void applySettings(List<Setting> items) {
    }
}
