package com.example.knot.ccallers.billing;

import com.example.knot.ccallers.ledger.Ledger;

public class Invoicer {
    private final Ledger ledger;

    public Invoicer(Ledger ledger) {
        this.ledger = ledger;
    }

    public void bill(String invoiceId) {
        ledger.post(invoiceId);
    }
}
