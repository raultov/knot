package com.example.knot.idispatch.web;

import com.example.knot.idispatch.service.LookupGateway;
import com.example.knot.idispatch.service.LookupOptions;
import lombok.RequiredArgsConstructor;

@RestController
@RequiredArgsConstructor
public class LookupGatewayController {
    private final LookupGateway lookupGateway;

    @GetMapping("/options")
    public LookupOptions optionsViaThisField() {
        return this.lookupGateway.listOptions();
    }
}
