package com.example.knot.http;

/** A declarative client: its mappings are calls it makes, not endpoints. */
@FeignClient(name = "owners")
public interface RemoteOwnersClient {
    @GetMapping("/remote/owners")
    List<Owner> owners();
}
