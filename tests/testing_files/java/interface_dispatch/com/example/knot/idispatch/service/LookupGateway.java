package com.example.knot.idispatch.service;

/**
 * Interface whose implementation lives in the same package and whose name is a
 * suffix of the implementation name (`DefaultLookupGateway`), see issue #29.
 */
@Validated
public interface LookupGateway {
    LookupOptions listOptions();
}
