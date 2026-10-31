package com.example.items;

@RestController
public class HealthController {
    @GetMapping("/health")
    public String health() {
        return "UP";
    }
}
