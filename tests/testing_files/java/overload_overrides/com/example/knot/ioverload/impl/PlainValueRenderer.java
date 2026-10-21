package com.example.knot.ioverload.impl;

import com.example.knot.ioverload.api.ValueRenderer;

public class PlainValueRenderer implements ValueRenderer {
    @Override
    public String renderValue(String value) {
        return value;
    }
}
