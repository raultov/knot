package com.example.knot.ioverload.api;

import java.util.Map;

/**
 * Generic supertype: `storeEntry(T entry)` is overridden by
 * `storeEntry(Setting entry)` in `EntryStore<Setting>` implementations, while
 * the `Map<String, T>` overload keeps its own implementation.
 */
public interface EntryStore<T> {
    void storeEntry(T entry);

    void storeEntry(Map<String, T> entries);

    default void storeEntry(String key, T entry) {
        storeEntry(entry);
    }
}
