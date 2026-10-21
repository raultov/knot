package com.example.knot.ioverload.impl;

import com.example.knot.ioverload.api.Setting;
import com.example.knot.ioverload.api.SettingsService;
import java.util.List;

@Service
public class DefaultSettingsService implements SettingsService {
    @Override
    public void applySettings(List<Setting> items, boolean skipValidation) {
    }
}
