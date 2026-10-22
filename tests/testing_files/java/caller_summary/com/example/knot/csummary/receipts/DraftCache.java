package com.example.knot.csummary.receipts;

/**
 * Unrelated homonym `persist` with callers of its own: they must never be
 * attributed to `ReceiptArchive.persist` (issue #31).
 */
public class DraftCache {
    public void cacheDraft(Receipt receipt) {
        persist(receipt);
    }

    public void refreshDraft(Receipt receipt) {
        persist(receipt);
    }

    void persist(Receipt receipt) {
        // keep the draft in memory
    }
}
