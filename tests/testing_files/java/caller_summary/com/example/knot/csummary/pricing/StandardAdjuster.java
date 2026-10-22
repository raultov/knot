package com.example.knot.csummary.pricing;

import java.math.BigDecimal;

/**
 * Shares the private helper name `computeValue` with `AlternateAdjuster`
 * (issue #31): the search caller summary must only count callers bound to
 * this class's helper.
 */
public class StandardAdjuster {
    public BigDecimal adjust(AdjustmentRequest request) {
        return computeValue(request);
    }

    /** Compute the standard price adjustment value for a request. */
    private BigDecimal computeValue(AdjustmentRequest request) {
        return BigDecimal.ONE;
    }
}
