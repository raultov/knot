package com.example.knot.idispatch.ledger;

@Repository
public interface OrderLedger {
    LedgerEntry appendEntry(LedgerEntry entry);
}
