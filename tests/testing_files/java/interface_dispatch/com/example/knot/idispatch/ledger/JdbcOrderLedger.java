package com.example.knot.idispatch.ledger;

@Repository
public class JdbcOrderLedger implements OrderLedger {
    @Override
    public LedgerEntry appendEntry(LedgerEntry entry) {
        return entry;
    }
}
