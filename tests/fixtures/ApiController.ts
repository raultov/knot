/**
 * REST API controller for handling HTTP requests.
 * Demonstrates routing, method decorators, and service dependencies.
 */

import { UserService } from "./UserService";

/**
 * HTTP controller for managing user endpoints.
 */
export class ApiController {
    private userService: UserService;

    constructor(userService: UserService) {
        this.userService = userService;
    }

    /**
     * GET /users/:id
     * Retrieve a user by ID.
     */
    getUser(userId: string): any {
        const user = this.userService.getUser(userId);
        if (!user) {
            return { error: "User not found" };
        }
        return { data: user };
    }

    /**
     * POST /users
     * Create a new user.
     */
    createUser(userData: any): any {
        try {
            this.userService.createUser(userData);
            return { success: true, data: userData };
        } catch (error) {
            return { error: String(error) };
        }
    }

    /**
     * PUT /users/:id
     * Update an existing user.
     */
    updateUser(userId: string, updates: any): any {
        try {
            this.userService.updateUser(userId, updates);
            return { success: true };
        } catch (error) {
            return { error: String(error) };
        }
    }

    /**
     * DELETE /users/:id
     * Delete a user.
     */
    deleteUser(userId: string): any {
        const success = this.userService.deleteUser(userId);
        return { success };
    }
}
