package com.example.knot.ifields.web;

import com.example.knot.ifields.api.CatalogProvider;
import java.util.List;

/**
 * Field name is the decapitalized interface name: resolves today (guard).
 */
public class MatchingFieldReportService {
    private final CatalogProvider catalogProvider;

    public MatchingFieldReportService(CatalogProvider catalogProvider) {
        this.catalogProvider = catalogProvider;
    }

    public List<String> buildWithMatchingField() {
        return catalogProvider.fetchItems();
    }
}
