package com.example.knot.ioverload.web;

import com.example.knot.ioverload.api.Setting;
import com.example.knot.ioverload.api.SettingsService;
import java.util.List;

public class SettingsController {
    private SettingsService settingsService;

    public void refreshWithValidation(List<Setting> items) {
        settingsService.applySettings(items);
    }
}
