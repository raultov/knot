package com.example.knot.ioverload.api;

/**
 * Same-named overloads with the same arity: the parameter types decide which
 * declaration an implementation overrides.
 */
public interface ValueRenderer {
    String renderValue(String value);

    default String renderValue(Long value) {
        return renderValue(String.valueOf(value));
    }
}
