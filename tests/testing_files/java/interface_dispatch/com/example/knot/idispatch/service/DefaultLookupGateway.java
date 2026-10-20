package com.example.knot.idispatch.service;

import org.springframework.stereotype.Service;

@Service
public class DefaultLookupGateway implements LookupGateway {
    @Override
    public LookupOptions listOptions() {
        return new LookupOptions();
    }
}
