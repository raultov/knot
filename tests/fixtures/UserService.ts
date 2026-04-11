/**
 * User service for handling user-related operations.
 * Demonstrates TypeScript class patterns, methods, and type references.
 */

interface User {
    id: string;
    name: string;
    email: string;
}

/**
 * Service class for managing users.
 */
export class UserService {
    private users: Map<string, User> = new Map();

    /**
     * Create a new user.
     * @param user The user data to create
     */
    createUser(user: User): void {
        if (!user.id || !user.name) {
            throw new Error("User must have id and name");
        }
        this.users.set(user.id, user);
    }

    /**
     * Get a user by ID.
     * @param userId The user ID
     */
    getUser(userId: string): User | undefined {
        return this.users.get(userId);
    }

    /**
     * Update an existing user.
     */
    updateUser(userId: string, updates: Partial<User>): void {
        const user = this.users.get(userId);
        if (!user) {
            throw new Error("User not found");
        }
        Object.assign(user, updates);
    }

    /**
     * Delete a user by ID.
     */
    deleteUser(userId: string): boolean {
        return this.users.delete(userId);
    }

    /**
     * Get all users.
     */
    getAllUsers(): User[] {
        return Array.from(this.users.values());
    }
}
