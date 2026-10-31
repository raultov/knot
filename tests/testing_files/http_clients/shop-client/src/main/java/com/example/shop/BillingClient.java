package com.example.shop;

/** OpenFeign client whose URL is a configuration property. */
@FeignClient(name = "billing", url = "${billing.url}")
public interface BillingClient {
    @PostMapping("/invoices")
    Invoice createInvoice(@RequestBody Invoice invoice);
}
