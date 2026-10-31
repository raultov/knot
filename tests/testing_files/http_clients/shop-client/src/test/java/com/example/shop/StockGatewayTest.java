package com.example.shop;

/** Test code: its HTTP calls are never linked. */
class StockGatewayTest {
    private TestRestTemplate testRestTemplate;

    void stockIsServed() {
        testRestTemplate.getForObject("/items/{id}/stock", Integer.class, 1);
    }
}
