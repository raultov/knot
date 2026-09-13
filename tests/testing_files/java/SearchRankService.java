package com.example.knot.test;

/**
 * Fixture for the search-ranking E2E suite. Deliberately models the
 * reported failure mode: {@code login} carries no Javadoc, so its behaviour
 * is only observable through the callees in its body, while the helpers
 * carry prose-rich doc comments full of "email" / "password" vocabulary.
 */
public class SearchRankService {

    /**
     * Normalizes an email address before it reaches the credentials check:
     * trims whitespace and lowercases so it matches the stored value.
     */
    public String normalizeEmail(String email) {
        return email.trim().toLowerCase();
    }

    /**
     * Verifies the plaintext password the user typed against the stored
     * hash and authenticates the caller.
     */
    public boolean verifyCredentialsOrFail(String email, String password) {
        return !email.isEmpty() && !password.isEmpty();
    }

    /**
     * Issues a fresh session token once the credentials are verified.
     */
    public String generateSessionToken(String email) {
        return "token-for-" + email;
    }

    // POST /api/v1/login handler — no Javadoc on purpose: this is the
    // fixture's definition-under-test.
    public String login(String email, String password) {
        String normalized = normalizeEmail(email);
        if (!verifyCredentialsOrFail(normalized, password)) {
            throw new IllegalArgumentException("invalid credentials");
        }
        return generateSessionToken(normalized);
    }
}
