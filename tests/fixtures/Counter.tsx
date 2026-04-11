/**
 * React Counter Component
 * Demonstrates TSX syntax with JSX, hooks, and component patterns.
 */

import React, { useState, useCallback } from "react";

interface CounterProps {
    initialValue?: number;
    onCountChange?: (count: number) => void;
}

/**
 * Counter component that maintains internal state.
 */
export const Counter: React.FC<CounterProps> = ({
    initialValue = 0,
    onCountChange,
}) => {
    const [count, setCount] = useState(initialValue);

    /**
     * Increment the counter.
     */
    const increment = useCallback(() => {
        const newCount = count + 1;
        setCount(newCount);
        onCountChange?.(newCount);
    }, [count, onCountChange]);

    /**
     * Decrement the counter.
     */
    const decrement = useCallback(() => {
        const newCount = count - 1;
        setCount(newCount);
        onCountChange?.(newCount);
    }, [count, onCountChange]);

    /**
     * Reset counter to initial value.
     */
    const reset = useCallback(() => {
        setCount(initialValue);
        onCountChange?.(initialValue);
    }, [initialValue, onCountChange]);

    return (
        <div className="counter">
            <div className="counter__display">Count: {count}</div>
            <button className="counter__button" onClick={increment}>
                Increment
            </button>
            <button className="counter__button" onClick={decrement}>
                Decrement
            </button>
            <button className="counter__button" onClick={reset}>
                Reset
            </button>
        </div>
    );
};

export default Counter;
