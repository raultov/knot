package com.example.items;

@RestController
@RequestMapping("/items")
public class ItemsController {
    @GetMapping("/{id}")
    public Item read(@PathVariable long id) {
        return new Item(id);
    }

    @PostMapping
    public Item create(@RequestBody Item item) {
        return item;
    }

    @GetMapping("/export")
    public String export() {
        return "csv";
    }

    @GetMapping("/{id}/stock")
    public int stock(@PathVariable long id) {
        return 3;
    }

    @PutMapping("/{id}")
    public Item update(@PathVariable long id, @RequestBody Item item) {
        return item;
    }

    @DeleteMapping("/{id}")
    public void delete(@PathVariable long id) {
    }
}
