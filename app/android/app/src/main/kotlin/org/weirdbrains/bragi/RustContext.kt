package org.weirdbrains.bragi

import android.content.Context

/**
 * Hands the application Context to the Rust bridge (ndk-context), which the networking
 * stack needs to read Android's DNS servers. Safe to call more than once. (From Morph.)
 */
object RustContext {
    private var initialized = false

    fun init(context: Context) {
        if (initialized) return
        System.loadLibrary("bragi_bridge")
        nativeInit(context.applicationContext)
        initialized = true
    }

    @JvmStatic
    private external fun nativeInit(context: Context)
}
