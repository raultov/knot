package com.example.knot.ifields.web;

import com.example.knot.ifields.api.CatalogProvider;
import java.util.List;
import java.util.concurrent.CompletableFuture;

/**
 * Field name does not match the interface name: the call edge is dropped (issue #28).
 */
public class RenamedFieldReportService {
    private final CatalogProvider source;

    public RenamedFieldReportService(CatalogProvider source) {
        this.source = source;
    }

    public List<String> buildWithRenamedField() {
        return source.fetchItems();
    }

    public List<String> buildWithThisRenamedField() {
        return this.source.fetchItems();
    }

    public CompletableFuture<List<String>> scheduleGroupsViaMethodRef() {
        return CompletableFuture.supplyAsync(source::fetchGroups);
    }
}
