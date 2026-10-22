package com.example.knot.csummary.receipts;

/**
 * Calls the archive through the concrete type, so `ReceiptArchive.persist`
 * has no direct caller, only a dispatched one.
 */
public class ReceiptClient {
    private final JdbcReceiptArchive archive = new JdbcReceiptArchive();

    public void archiveViaConcreteType(Receipt receipt) {
        archive.persist(receipt);
    }
}
