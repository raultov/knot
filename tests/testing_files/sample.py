#!/usr/bin/env python3
"""
Sample Python file for knot E2E testing (v0.8.12 - Phase 8 Phase 1)
Tests only PythonClass, PythonFunction, PythonMethod — the entities covered by Phase 1.
"""


class User:
    """Represents a user in the system."""

    def __init__(self, name: str, email: str):
        self.name = name
        self.email = email

    def greet(self) -> str:
        """Return a greeting message."""
        return f"Hello, {self.name}!"

    def get_email(self) -> str:
        return self.email


class Admin:
    """Administrator user with elevated privileges."""

    def __init__(self, name: str, role: str):
        self.name = name
        self.role = role

    def manage_users(self) -> list:
        return []


def process_data(data: list) -> int:
    """Process a list of items and return the count."""
    return len(data)


def fetch_users():
    """Fetch all users from the database."""
    return []


def main():
    """Main entry point."""
    users = fetch_users()
    for user in users:
        print(user.greet())


if __name__ == "__main__":
    main()
