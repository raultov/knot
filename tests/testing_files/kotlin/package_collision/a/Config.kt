package com.example.kpkg.a

interface Source {
    fun load(): String
}

class Config : Source {
    override fun load(): String = "a"
}

class LoaderA {
    fun readA(config: Config): String = config.load()
}
