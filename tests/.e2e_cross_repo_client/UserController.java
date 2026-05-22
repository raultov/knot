package com.example;

public class UserController {
    private AuthService authService = new AuthService();

    public void handleLogin(String user, String pass) {
        boolean ok = authService.login(user, pass);
        if (ok) {
            System.out.println("Login successful");
        }
    }
}
