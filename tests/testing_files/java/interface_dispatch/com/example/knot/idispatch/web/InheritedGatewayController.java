package com.example.knot.idispatch.web;

import com.example.knot.idispatch.service.LookupOptions;

/**
 * The receiver is a field inherited from a superclass declared in another
 * file, so the parser cannot type it and the resolver falls back to the
 * receiver name (issue #29).
 */
@RestController
public class InheritedGatewayController extends AbstractGatewayController {
    public LookupOptions optionsViaInheritedField() {
        return lookupGateway.listOptions();
    }
}
