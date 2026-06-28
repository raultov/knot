#!/usr/bin/env python3
"""
Sample Python file for knot E2E testing (v0.9.0 - Phase 4)
Tests PythonClass, PythonFunction, PythonMethod, PythonConstant,
CALLS relationships, and REFERENCES (imports).
"""

import os
from sys import argv

MAX_RETRIES = 5
DEFAULT_TIMEOUT = 30


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


def validate_email(email: str) -> bool:
    """Validate an email address."""
    return "@" in email


def main():
    """Main entry point."""
    users = fetch_users()
    for user in users:
        greeting = user.greet()
        print(greeting)
        email = user.get_email()
        is_valid = validate_email(email)
        if is_valid:
            print(f"Valid email: {email}")


if __name__ == "__main__":
    main()


# Phase 4.5: ValueReferences - Classes/functions used as keyword argument values
# Mirrors ComfyUI cli_args.py pattern: EnumAction used as action=EnumAction
# after a class with a classmethod that does NOT use EnumAction

import argparse

parser = argparse.ArgumentParser()

class CustomAction:
    """Custom argparse action for testing value references."""
    pass

# Simulates LatentPreviewMethod enum with a classmethod near the action= line
# (like from_string at line 103 in cli_args.py, action=EnumAction at line 109)
class Status:
    VALID = "valid"
    INVALID = "invalid"

    @classmethod
    def from_string(cls, value: str):
        for member in cls:
            if member.value == value:
                return member
        return None

parser.add_argument("--status", action=CustomAction, help="Custom action test")

def custom_callback():
    """Custom callback for testing value references."""
    pass


def register_handler(callback=None):
    """Register a callback handler."""
    pass


register_handler(callback=custom_callback)


# ============================================================
# Phase 5: Inheritance (EXTENDS) and Decorators (CALLS)
# ============================================================

class Animal:
    """Base class for animals."""

    def speak(self) -> str:
        return "..."

class Dog(Animal):
    """A dog that extends Animal."""

    def speak(self) -> str:
        return "Woof!"

class Cat(Animal):
    """A cat that extends Animal."""

    def speak(self) -> str:
        return "Meow!"

from dataclasses import dataclass

@dataclass
class Point:
    """A point with x and y coordinates."""
    x: int
    y: int


class MathUtils:
    """Utility class with static methods."""

    @staticmethod
    def add(a: int, b: int) -> int:
        return a + b

    @staticmethod
    def multiply(a: int, b: int) -> int:
        return a * b


class Service:
    """A service with multiple decorators."""

    @staticmethod
    @property
    def version() -> str:
        return "1.0"


# ============================================================
# Phase 6: Advanced Type Hints, *args/**kwargs, Py2 Syntax
# ============================================================

from typing import List, Dict, Optional

def process_items(items: List[str], config: Dict[str, int]) -> Dict[str, int]:
    """Process a list of items with configuration.
    
    Uses generic type hints for input validation.
    """
    return {}


def find_user(user_id: int) -> Optional[Dict[str, str]]:
    """Find a user by ID, returning None if not found.
    
    Demonstrates Optional return type annotation.
    """
    return None


def log_message(message: str, *args, level: str = "INFO", **kwargs):
    """Log a message with variable arguments.
    
    Demonstrates *args and **kwargs unpacking.
    """
    pass


def handle_exception_modern():
    """Demonstrate Python 3 exception syntax."""
    try:
        raise ValueError("bad value")
    except ValueError as e:
        pass


def handle_exception_py2_style():
    """Demonstrate Python 2 compatible exception syntax (py2)."""
    try:
        raise ValueError("bad value")
    except ValueError, e:
        pass


# ============================================================
# Python bug-fix: self.method() resolution (resolves locally first)
# ============================================================

class Calculator:
    """A calculator with self-calling methods."""

    def add(self, a: int, b: int) -> int:
        return a + b

    def multiply(self, a: int, b: int) -> int:
        return a * b

    def compute_sum_product(self, a: int, b: int) -> tuple:
        """Calls self.add() and self.multiply() — should resolve locally."""
        s = self.add(a, b)
        p = self.multiply(a, b)
        return (s, p)


# ============================================================
# Regression test: self.method() with name collision (ComfyUI load_lora bug)
# ============================================================
# Scenario: module-level function has same name as a class method.
# self.method() MUST resolve to the local class method, NOT the module-level one.

def lib_load_lora(model, clip, lora_name, strength_model, strength_clip):
    """Module-level function — simulates comfy/lora.py:load_lora."""
    pass


class MyLoraLoader:
    """Simulates the LoraLoader class in nodes_lora_debug.py."""

    def lib_load_lora(self, model, clip, lora_name, strength_model, strength_clip):
        """Class method with same name as module-level function — different entity."""
        pass

    def load_lora_model_only(self, model, lora_name, strength):
        """Calls self.lib_load_lora() — should resolve to THIS class's method."""
        return (self.lib_load_lora(model, None, lora_name, strength, 0)[0],)


# ============================================================
# Regression: ClassName(...) must resolve to ClassName.__init__
# Spec: docs/specs/python_constructor_call_resolution.md
# ============================================================

class MuonLike:
    def __init__(self, lr, wd):
        self.lr = lr
        self.wd = wd

def create_muon_like():
    return MuonLike(lr=1e-3, wd=0.1)   # caller esperado de MuonLike.__init__


# ============================================================
# Regression: super().__init__() must resolve to the parent
# class's __init__, not the enclosing class's own __init__.
# Real-world report: LlamaFactory webui/chatter.py::WebChatModel.
# ============================================================

class BaseEngineLike:
    def __init__(self, name: str = "base"):
        self.name = name


class WebChatLike(BaseEngineLike):
    def __init__(self, lazy_init: bool = True):
        self.lazy_init = lazy_init
        if not lazy_init:
            super().__init__()                 # caller esperado de BaseEngineLike.__init__
        if not lazy_init:
            super().__init__(name="explicit")  # caller esperado de BaseEngineLike.__init__

    @property
    def loaded(self) -> bool:
        return self.lazy_init is False

    def load_model_chained(self, data) -> str:
        return "loaded:" + str(data)


# Module-level homonym for `load_model_chained` — this forces ambiguity that
# only the receiver-chain disambiguator can resolve. Without the disambiguator,
# the chained call below would have two candidates and be dropped.
def load_model_chained(data) -> str:
    return "module-level:" + str(data)


class WebChatHost:
    def __init__(self):
        self.chatter = WebChatLike()

    def build_ui(self, click_handler):
        # `engine.chatter.load_model_chained(...)` — chained call. The receiver
        # `self.chatter` must steer the resolver to WebChatLike.load_model_chained,
        # not the module-level homonym.
        self.chatter.load_model_chained("hi")

        # `engine.chatter.loaded` — chained attribute access used as a value.
        # Must emit a ValueReference for the trailing identifier `loaded`.
        info = {"visible": self.chatter.loaded}

        # `click_handler(self.chatter.load_model_chained, ...)` — method passed
        # as a value (Gradio-style callback). Must emit a ValueReference for
        # the trailing identifier `load_model_chained`.
        click_handler(self.chatter.load_model_chained, info)

        return info
