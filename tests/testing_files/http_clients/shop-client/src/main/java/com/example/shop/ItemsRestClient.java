package com.example.shop;

/** MicroProfile REST client; its base URL comes from the configuration. */
@RegisterRestClient(configKey = "items")
@Path("/items")
public interface ItemsRestClient {
    @POST
    Item create(Item item);
}
