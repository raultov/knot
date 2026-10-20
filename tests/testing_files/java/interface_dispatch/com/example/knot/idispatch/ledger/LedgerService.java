package com.example.knot.idispatch.ledger;

@Service
public class LedgerService {
    private final OrderLedger orderLedger;

    public LedgerService(OrderLedger orderLedger) {
        this.orderLedger = orderLedger;
    }

    public LedgerEntry recordLedgerEntry(LedgerEntry entry) {
        return this.orderLedger.appendEntry(entry);
    }
}
