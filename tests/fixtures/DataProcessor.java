package com.example.data;

/**
 * A class demonstrating inheritance and cross-repository dependency patterns.
 * This class extends a base class to show inheritance relationships.
 */
public class DataProcessor extends BaseProcessor implements Processor {
    
    private String name;
    private int batchSize;
    
    /**
     * Constructor initializing the processor.
     */
    public DataProcessor(String name, int batchSize) {
        this.name = name;
        this.batchSize = batchSize;
    }
    
    /**
     * Process data with the given input.
     * Demonstrates a call to another class method.
     */
    @Override
    public void process(String input) {
        validate(input);
        performProcessing(input);
    }
    
    /**
     * Validate input data.
     */
    private void validate(String input) {
        if (input == null) {
            throw new IllegalArgumentException("Input cannot be null");
        }
    }
    
    /**
     * Perform the actual data processing.
     */
    private void performProcessing(String input) {
        // Process the input
    }
    
    /**
     * Get the processor name.
     */
    public String getName() {
        return name;
    }
    
    /**
     * Get the batch size.
     */
    public int getBatchSize() {
        return batchSize;
    }
}
