package com.example.knot.ifields.api;

import java.util.List;

/**
 * Interface injected into services through interface-typed fields (issue #28).
 */
public interface CatalogProvider {
    List<String> fetchItems();

    List<String> fetchGroups();
}
