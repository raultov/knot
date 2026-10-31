package com.example.shop;

/** RestTemplate calls whose base URL is a configuration property. */
@Service
public class StockGateway {
    private final RestTemplate restTemplate;
    private final Map<Long, Integer> cache = new HashMap<>();

    @Value("${items.base-url}")
    private String itemsUrl;

    public StockGateway(RestTemplate restTemplate) {
        this.restTemplate = restTemplate;
    }

    public int stock(long id) {
        Integer level = restTemplate.getForObject(itemsUrl + "/items/{id}/stock", Integer.class, id);
        cache.put(id, level);
        return level;
    }

    public void remove(long id) {
        cache.remove(id);
        restTemplate.delete(itemsUrl + "/items/" + id);
    }
}
