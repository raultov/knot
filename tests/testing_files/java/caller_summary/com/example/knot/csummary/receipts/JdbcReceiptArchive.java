package com.example.knot.csummary.receipts;

public class JdbcReceiptArchive implements ReceiptArchive {
    @Override
    public void persist(Receipt receipt) {
        // INSERT INTO receipts ...
    }
}
