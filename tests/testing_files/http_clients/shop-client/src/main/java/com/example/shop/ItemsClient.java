package com.example.shop;

/** OpenFeign client of the items service, found through its service name. */
@FeignClient(name = "items-service", path = "/api/items")
public interface ItemsClient {
    @GetMapping("/{id}")
    Item fetch(@PathVariable("id") long id);
}
