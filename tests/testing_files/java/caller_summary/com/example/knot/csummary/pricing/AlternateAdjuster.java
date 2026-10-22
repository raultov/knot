package com.example.knot.csummary.pricing;

import java.math.BigDecimal;
import java.util.Map;

/**
 * Homonym of `StandardAdjuster.computeValue` with two callers of its own, so
 * a name-based caller summary over-counts both helpers (issue #31).
 */
public class AlternateAdjuster {
    public Map<String, BigDecimal> adjust(AdjustmentRequest request) {
        return computeValue(request);
    }

    public Map<String, BigDecimal> preview(AdjustmentRequest request) {
        return computeValue(request);
    }

    /** Compute the alternate per-currency adjustment values for a request. */
    private Map<String, BigDecimal> computeValue(AdjustmentRequest request) {
        return Map.of(request.getSku(), BigDecimal.TEN);
    }
}
