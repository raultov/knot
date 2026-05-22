package com.example;

public class AuthService {
    public boolean login(String username, String password) {
        return username != null && !username.isEmpty();
    }

    public void logout(String username) {
        System.out.println("User " + username + " logged out");
    }
}
