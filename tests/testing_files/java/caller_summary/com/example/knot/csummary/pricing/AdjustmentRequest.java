package com.example.knot.csummary.pricing;

public class AdjustmentRequest {
    private final String sku;

    public AdjustmentRequest(String sku) {
        this.sku = sku;
    }

    public String getSku() {
        return sku;
    }
}
