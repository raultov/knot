package com.example.kpkg.app

import com.example.kpkg.a.Config as AConfig
import com.example.kpkg.b.Config

class Consumer {
    fun useB(config: Config): String = config.load()

    fun useA(aConfig: AConfig): String = aConfig.load()
}
