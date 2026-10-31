package com.example.knot.incr;

public class Auditor {
    private final Ledger ledger = new Ledger();

    public void check(String entry) {
        ledger.audit(entry);
    }
}
