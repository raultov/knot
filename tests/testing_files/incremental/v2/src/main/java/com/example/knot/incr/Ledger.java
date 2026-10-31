package com.example.knot.incr;

/** Append-only ledger: post moved down, audit removed. */
public class Ledger {
    private final java.util.List<String> entries = new java.util.ArrayList<>();

    /** Records one entry. */
    public void post(String entry) {
        entries.add(entry);
    }
}
