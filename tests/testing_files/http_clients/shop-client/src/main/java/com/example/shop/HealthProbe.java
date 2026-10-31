package com.example.shop;

/** Base URL known only at runtime: /health is served by two services. */
public class HealthProbe {
    private final RestTemplate restTemplate = new RestTemplate();

    public String probe(String baseUrl) {
        return restTemplate.getForObject(baseUrl + "/health", String.class);
    }
}
