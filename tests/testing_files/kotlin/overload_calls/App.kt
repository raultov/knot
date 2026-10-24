package com.example.kocalls.app

import com.example.kocalls.x.Child
import com.example.kocalls.x.Foo
import com.example.kocalls.x.Mapper
import com.example.kocalls.x.Padder

class App {
    fun sumPair(foo: Foo): Int = foo.bar(1, 2)

    fun single(foo: Foo): Int = foo.bar(1)

    fun scalePair(child: Child): Int = child.scale(2, 3)

    fun mapped(mapper: Mapper): Int = mapper.transform(1) { it * 2 }

    fun plain(mapper: Mapper): Int = mapper.transform(1)

    fun padShort(padder: Padder): String = padder.pad("x")

    fun padWide(padder: Padder): String = padder.pad("x", 4)
}
