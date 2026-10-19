package com.example.knot.ifields.impl;

import com.example.knot.ifields.api.CatalogProvider;
import java.util.List;

public class HttpCatalogProvider implements CatalogProvider {
    @Override
    public List<String> fetchItems() {
        return List.of();
    }

    @Override
    public List<String> fetchGroups() {
        return List.of();
    }
}
