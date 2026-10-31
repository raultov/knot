package com.example.shop

/** RestClient call whose base URL is unknown: treated as scheme and host. */
class InventoryClient(private val restClient: RestClient, private val baseUrl: String) {
    fun update(id: Long, item: Item) {
        restClient.put()
            .uri("$baseUrl/api/items/$id")
            .body(item)
            .retrieve()
            .toBodilessEntity()
    }
}
