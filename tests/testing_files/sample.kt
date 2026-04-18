// Sample Kotlin file for testing Kotlin support in knot indexer
package com.example

import kotlin.random.Random
import androidx.appcompat.app.AppCompatActivity

/**
 * Service class for handling user data operations
 */
@Service
class UserService {
    private val userRepository = UserRepository()
    
    fun getUser(id: Int): User? {
        return userRepository.findById(id)
    }
    
    fun saveUser(user: User): Boolean {
        return userRepository.save(user)
    }
}

// Interface declaration
interface Repository<T> {
    fun findById(id: Int): T?
    fun save(item: T): Boolean
}

// Implementation of interface
@Repository
class UserRepository : Repository<User> {
    private val users = mutableListOf<User>()
    
    override fun findById(id: Int): User? {
        return users.find { it.id == id }
    }
    
    override fun save(item: User): Boolean {
        users.add(item)
        return true
    }
}

// Data class
data class User(
    val id: Int,
    val name: String,
    val email: String
)

// Object declaration (singleton)
object DatabaseManager {
    fun connect(): Boolean {
        println("Connecting to database...")
        return true
    }
    
    fun disconnect() {
        println("Disconnecting from database...")
    }
}

// Companon object
class ConfigManager {
    companion object {
        const val DEFAULT_TIMEOUT = 5000
        val instance = ConfigManager()
        
        fun configure() {
            println("Configuring with timeout: $DEFAULT_TIMEOUT")
        }
    }
    
    fun loadData() {
        println("Loading configuration data...")
        val result = DatabaseManager.connect() // Calling object method
        val userService = UserService()       // Instantiation
        val user = userService.getUser(1)     // Method call
    }
}

// Top-level function
fun greetUser(user: User) {
    println("Hello, ${user.name}!")
    val randomValue = Random.nextInt(100) // External library usage
    println("Random value: $randomValue")
}

// Extension function
fun String.isValidEmail(): Boolean {
    return this.contains("@") && this.length > 5
}

// Usage of extension function
fun validateEmail(email: String) {
    if (email.isValidEmail()) {
        println("Valid email: $email")
    } else {
        println("Invalid email: $email")
    }
}

// Main function
fun main() {
    val user = User(1, "John Doe", "john@example.com")
    greetUser(user)
    
    val isValid = user.email.isValidEmail() // Using extension function
    println("Email is valid: $isValid")
    
    ConfigManager.configure() // Using companion object
    DatabaseManager.connect() // Using singleton object
}