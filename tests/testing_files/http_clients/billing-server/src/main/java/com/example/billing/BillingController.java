package com.example.billing;

@RestController
public class BillingController {
    @PostMapping("/invoices")
    public Invoice createInvoice(@RequestBody Invoice invoice) {
        return invoice;
    }

    @GetMapping("/invoices/{number}")
    public Invoice readInvoice(@PathVariable String number) {
        return new Invoice(number);
    }

    @PostMapping("/refunds")
    public Refund refund(@RequestBody Refund refund) {
        return refund;
    }

    @GetMapping("/health")
    public String health() {
        return "UP";
    }
}
