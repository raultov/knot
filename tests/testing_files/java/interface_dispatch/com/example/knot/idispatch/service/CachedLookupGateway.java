package com.example.knot.idispatch.service;

@Service
public class CachedLookupGateway implements LookupGateway {
    @Override
    public LookupOptions listOptions() {
        return new LookupOptions();
    }
}
