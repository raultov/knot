package com.example.knot.ccallers.billing;

import com.example.knot.ccallers.ledger.Ledger;

public class Refunder {
    private final Ledger ledger;

    public Refunder(Ledger ledger) {
        this.ledger = ledger;
    }

    public void refund(String refundId) {
        ledger.post(refundId);
    }
}
