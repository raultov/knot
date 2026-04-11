package com.example.service;

import com.example.utils.Logger;
import java.util.List;

/**
 * A simple service class for testing entity extraction.
 * This demonstrates basic Java patterns: classes, methods, and calls.
 */
public class SimpleService {
    private static final Logger logger = Logger.getInstance();
    
    /**
     * Initialize the service with configuration.
     * @param config the service configuration
     */
    public void initialize(String config) {
        logger.info("Initializing service with config: " + config);
        validateConfig(config);
    }
    
    /**
     * Validate the provided configuration string.
     */
    private void validateConfig(String config) {
        if (config == null || config.isEmpty()) {
            throw new IllegalArgumentException("Config cannot be empty");
        }
    }
    
    /**
     * Process a list of items.
     */
    public List<String> processItems(List<String> items) {
        logger.debug("Processing " + items.size() + " items");
        return items;
    }
    
    /**
     * Static utility method.
     */
    public static void staticMethod() {
        logger.info("Static method called");
    }
}
