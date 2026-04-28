package com.example.service

import groovy.transform.CompileStatic
import groovy.util.logging.Slf4j
import javax.inject.Singleton

// 1. Top-level script variables and closures
def globalConfig = [environment: "prod", version: "1.0"]
def processDataClosure = { List<String> data ->
    data.each { println it }
}

// 2. Top-level script methods
def scriptMethod(String input) {
    return input.toUpperCase()
}

String anotherScriptMethod() {
    return "Hello from script"
}

// 3. Generic Interface
interface Repository<T> {
    T findById(String id)
    List<T> findAll()
}

// 4. Trait with state and default methods
trait Auditable {
    String auditUser = "system"

    def logAction(String action) {
        println "Audit [${auditUser}]: ${action}"
    }
}

// 5. Enum
enum Status {
    ACTIVE, INACTIVE, DELETED, PENDING
}

// 6. Abstract Base Class
abstract class BaseService {
    protected String environment

    BaseService(String environment) {
        this.environment = environment
    }

    abstract void initialize()
}

// 7. Main Class with Annotations, Inheritance, Traits, and inner classes
@Slf4j
@Singleton
@CompileStatic
class UserService extends BaseService implements Repository<String>, Auditable {
    
    // Properties with visibility
    public static final String DEFAULT_ROLE = "USER"
    private int maxLoginAttempts = 5
    String serviceName = "UserService"

    // Constructor calling super()
    UserService() {
        super("production")
    }

    // Typed Method overriding base class
    @Override
    void initialize() {
        log.info("Initializing ${serviceName} in ${environment}")
    }

    // Interface Implementations
    @Override
    String findById(String id) {
        logAction("Finding user ${id}") // Call to trait method
        return "user_${id}"
    }

    @Override
    List<String> findAll() {
        return ["user_1", "user_2"]
    }

    // Untyped (def) method
    def calculateTotal(int a, int b) {
        def result = a + b
        return result
    }

    // 8. Static inner class
    static class DatabaseConfig {
        String url
        int port
    }
}

// 9. Spock Specification (parameterized test)
class CalculatorSpec extends Specification {
    
    @Feature
    @Unroll
    void "addition of #num1 and #num2 should be #expected"() {
        given: "a calculator instance"
        def calculator = new Calculator()
        
        when: "adding two numbers"
        def result = calculator.add(num1, num2)
        
        then: "the result is correct"
        result == expected
        
        where:
        num1 | num2 | expected
        3    | 5    | 8
        2    | 3    | 5
        7    | 4    | 11
        0    | 0    | 0
    }
}
