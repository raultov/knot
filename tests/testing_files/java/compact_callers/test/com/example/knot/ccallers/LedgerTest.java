package com.example.knot.ccallers;

import com.example.knot.ccallers.ledger.Ledger;

class LedgerTest {
    private final Ledger ledger = new Ledger();

    void postsEntry() {
        ledger.post("a");
    }

    void postsTwice() {
        ledger.post("a");
        ledger.post("b");
    }
}
