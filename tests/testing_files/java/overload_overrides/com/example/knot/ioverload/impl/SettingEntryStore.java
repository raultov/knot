package com.example.knot.ioverload.impl;

import com.example.knot.ioverload.api.EntryStore;
import com.example.knot.ioverload.api.Setting;
import java.util.Map;

public class SettingEntryStore implements EntryStore<Setting> {
    @Override
    public void storeEntry(Setting entry) {
    }

    @Override
    public void storeEntry(Map<String, Setting> entries) {
    }
}
