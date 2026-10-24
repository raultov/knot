package com.example.kocalls.x

class Foo {
    fun bar(a: Int, b: Int): Int = a + b

    fun bar(a: Int): Int = a
}

open class Base {
    fun scale(a: Int, b: Int): Int = a * b
}

class Child : Base() {
    fun scale(a: Int): Int = a
}

/** A trailing lambda is an argument: `transform(1) { it }` has two. */
class Mapper {
    fun transform(a: Int, f: (Int) -> Int): Int = f(a)

    fun transform(a: Int): Int = a
}

/** A default parameter accepts both arities. */
class Padder {
    fun pad(text: String, width: Int = 0): String = text
}
