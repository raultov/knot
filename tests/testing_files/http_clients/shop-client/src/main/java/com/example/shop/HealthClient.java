package com.example.shop;

/** Spring HTTP interface without a service: /health is served by two services. */
@HttpExchange("/health")
public interface HealthClient {
    @GetExchange
    String health();
}
