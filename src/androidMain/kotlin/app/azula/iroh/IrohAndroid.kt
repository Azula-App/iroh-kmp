package app.azula.iroh

import android.content.Context

/**
 * Android-only startup hook. iroh's DNS resolver needs the process JavaVM and
 * application context (via `ndk_context`) before any [IrohEndpoint] is bound, so
 * call this once at process startup:
 *
 * ```
 * IrohAndroid.installAndroidContext(applicationContext)
 * ```
 *
 * The native method lives in the same library as the generated UniFFI bindings.
 */
public object IrohAndroid {
    init {
        System.loadLibrary("iroh_kmp")
    }

    @JvmStatic
    public external fun installAndroidContext(context: Context)
}
