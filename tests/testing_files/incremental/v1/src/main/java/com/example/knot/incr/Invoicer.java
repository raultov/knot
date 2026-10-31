package com.example.knot.incr;

public class Invoicer {
    private final Ledger ledger = new Ledger();

    public void bill(String invoiceId) {
        ledger.post(invoiceId);
    }
}
