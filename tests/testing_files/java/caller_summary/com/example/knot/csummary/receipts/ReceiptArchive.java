package com.example.knot.csummary.receipts;

/**
 * Interface method with no direct caller: its only call is statically bound
 * to the implementation, so it is reached through dispatch (issue #31).
 */
public interface ReceiptArchive {
    /** Persist a receipt into the long-term archive. */
    void persist(Receipt receipt);
}
