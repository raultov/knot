package com.example.khttp

const val API = "/api"

@RestController
@RequestMapping("$API/article")
class ArticleController(private val repository: ArticleRepository) {

    @GetMapping("/")
    fun findAll() = repository.findAllByOrderByAddedAtDesc()

    @GetMapping(SLUG)
    fun findOne(@PathVariable slug: String) = repository.findBySlug(slug)

    @PostMapping(value = ["/drafts", "/published"])
    fun create(@RequestBody article: Article) = repository.save(article)

    companion object {
        const val SLUG = "/{slug}"
    }
}
