package com.example.kpkg.b

interface Source {
    fun load(): String
}

class Config : Source {
    override fun load(): String = "b"
}

class LoaderB {
    fun readB(config: Config): String = config.load()
}
