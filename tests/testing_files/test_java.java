// E2E Test File for Java Features
// This file tests Java annotation and dependency injection:
// - Annotations (@Service, @Component, @Autowired, @RestController)
// - Type references in constructor parameters (Spring DI)
// - Type references in field declarations
// - JavaDoc comments
// - Multiple classes in one file
// - Enum declarations

import java.util.List;
import java.util.ArrayList;

/**
 * Repository interface for data access.
 * 
 * @param <T> The entity type
 */
interface Repository<T> {
    T findById(Long id);
    List<T> findAll();
    void save(T entity);
}

/**
 * User entity class.
 * Tests: JavaDoc extraction, field declarations with types
 */
class User {
    private Long id;
    private String username;
    private String email;

    /**
     * Creates a new user with the given username and email.
     * 
     * @param username The username
     * @param email The email address
     */
    public User(String username, String email) {
        this.username = username;
        this.email = email;
    }

    /**
     * Get the user's ID
     * 
     * @return The user ID
     */
    public Long getId() {
        return id;
    }

    public String getUsername() {
        return username;
    }

    public String getEmail() {
        return email;
    }
}

/**
 * User repository implementation.
 * Tests: Annotation extraction (@Component), interface implementation
 */
@Component
class UserRepository implements Repository<User> {
    private List<User> users = new ArrayList<>();

    @Override
    public User findById(Long id) {
        return users.stream()
            .filter(u -> u.getId().equals(id))
            .findFirst()
            .orElse(null);
    }

    @Override
    public List<User> findAll() {
        return new ArrayList<>(users);
    }

    @Override
    public void save(User entity) {
        users.add(entity);
    }
}

/**
 * Email service for sending notifications.
 * Tests: @Service annotation, method with typed parameters
 */
@Service
class EmailService {
    /**
     * Send an email notification
     * 
     * @param to Recipient email address
     * @param subject Email subject
     * @param body Email body
     */
    public void sendEmail(String to, String subject, String body) {
        System.out.println("Sending email to: " + to);
    }

    /**
     * Send a welcome email to a new user
     * 
     * @param user The new user
     */
    public void sendWelcomeEmail(User user) {
        sendEmail(user.getEmail(), "Welcome!", "Welcome to our platform");
    }
}

/**
 * User service with dependency injection.
 * Tests: Constructor injection with @Autowired, type references in constructor params
 */
@Service
class UserService {
    private final UserRepository userRepository;
    private final EmailService emailService;

    /**
     * Constructor with dependency injection.
     * Should capture UserRepository and EmailService as type references.
     * 
     * @param userRepository The user repository
     * @param emailService The email service
     */
    @Autowired
    public UserService(UserRepository userRepository, EmailService emailService) {
        this.userRepository = userRepository;
        this.emailService = emailService;
    }

    /**
     * Register a new user
     * 
     * @param username The username
     * @param email The email address
     * @return The created user
     */
    public User registerUser(String username, String email) {
        User user = new User(username, email);
        userRepository.save(user);
        emailService.sendWelcomeEmail(user);
        return user;
    }

    /**
     * Get all users
     * 
     * @return List of all users
     */
    public List<User> getAllUsers() {
        return userRepository.findAll();
    }
}

/**
 * REST controller for user endpoints.
 * Tests: @RestController annotation, field injection with @Autowired
 */
@RestController
class UserController {
    /**
     * Injected user service.
     * Tests: Field-level dependency injection
     */
    @Autowired
    private UserService userService;

    /**
     * Get all users endpoint
     * 
     * @return List of users
     */
    @GetMapping("/users")
    public List<User> getUsers() {
        return userService.getAllUsers();
    }

    /**
     * Create user endpoint
     * 
     * @param request The user creation request
     * @return The created user
     */
    @PostMapping("/users")
    public User createUser(CreateUserRequest request) {
        return userService.registerUser(request.getUsername(), request.getEmail());
    }
}

/**
 * Request DTO for user creation
 */
class CreateUserRequest {
    private String username;
    private String email;

    public String getUsername() {
        return username;
    }

    public void setUsername(String username) {
        this.username = username;
    }

    public String getEmail() {
        return email;
    }

    public void setEmail(String email) {
        this.email = email;
    }
}

/**
 * User status enumeration.
 * Tests: Enum declaration and usage
 */
enum UserStatus {
    ACTIVE,
    INACTIVE,
    SUSPENDED,
    DELETED
}

/**
 * Configuration class.
 * Tests: @Configuration annotation, @Bean methods
 */
@Configuration
class AppConfig {
    /**
     * Create email service bean
     * 
     * @return Email service instance
     */
    @Bean
    public EmailService emailService() {
        return new EmailService();
    }

    /**
     * Create user repository bean
     * 
     * @return User repository instance
     */
    @Bean
    public UserRepository userRepository() {
        return new UserRepository();
    }
}

// Annotation placeholders (would be from Spring Framework in real code)
@interface Service {}
@interface Component {}
@interface RestController {}
@interface Autowired {}
@interface GetMapping { String value(); }
@interface PostMapping { String value(); }
@interface Configuration {}
@interface Bean {}
