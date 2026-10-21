package com.example.overloads

/**
 * Same-named overloads (issue #30): arity and parameter types decide which
 * declaration an override implements; type parameters match concrete types.
 */
interface EntryFormatter<T> {
    fun formatEntry(value: T): String

    fun formatEntry(value: T, width: Int): String = formatEntry(value)

    fun formatEntry(values: List<T>): String
}

class IntEntryFormatter : EntryFormatter<Int> {
    override fun formatEntry(value: Int): String = value.toString()

    override fun formatEntry(values: List<Int>): String = values.joinToString()
}

/** Function-typed parameters carry commas and `->` inside a single parameter. */
interface EventSink {
    fun onSinkEvent(handler: (String, Int) -> Unit)

    fun onSinkEvent(name: String, handler: (String, Int) -> Unit)
}

class ConsoleEventSink : EventSink {
    override fun onSinkEvent(handler: (String, Int) -> Unit) {}

    override fun onSinkEvent(name: String, handler: (String, Int) -> Unit) {}
}
