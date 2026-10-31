package com.example.shop;

public class CheckoutService {
    private final ItemsClient itemsClient;

    public CheckoutService(ItemsClient itemsClient) {
        this.itemsClient = itemsClient;
    }

    public Item checkout(long id) {
        return itemsClient.fetch(id);
    }
}
