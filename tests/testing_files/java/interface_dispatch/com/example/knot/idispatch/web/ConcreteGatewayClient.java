package com.example.knot.idispatch.web;

import com.example.knot.idispatch.service.CachedLookupGateway;
import com.example.knot.idispatch.service.DefaultLookupGateway;
import com.example.knot.idispatch.service.LookupOptions;

/**
 * Calls statically bound to implementations: they reach the interface method
 * only through dispatch, and must never leak into a sibling implementation.
 */
public class ConcreteGatewayClient {
    public LookupOptions optionsViaConcreteType() {
        DefaultLookupGateway concrete = new DefaultLookupGateway();
        return concrete.listOptions();
    }

    public LookupOptions optionsViaCachedGateway() {
        CachedLookupGateway cached = new CachedLookupGateway();
        return cached.listOptions();
    }
}
