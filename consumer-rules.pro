# R8/ProGuard rules shipped inside the AAR and applied automatically to every
# consumer's release build (see `consumerProguardFiles` in build.gradle.kts).
#
# Why this file has to exist: Gobley's UniFFI bindings call through JNA, and JNA
# resolves things from native code *by their literal source names* — via JNI
# `GetFieldID`/`GetMethodID` and by dlsym'ing symbol names off a `Library`
# interface. R8 has no way to see those uses, so by default it renames them and
# the lookups fail at runtime. The upstream `jna` AAR ships no consumer rules of
# its own, and Amper (what azula-app builds with) exposes no proguard/R8 config
# surface, so a consuming app cannot add these itself — the SDK that pulls JNA in
# is the only place they can live.
#
# The failure this prevents is silent at build time and fatal at runtime, in
# release builds only: R8 renames `com.sun.jna.Pointer.peer` to something short,
# `Native.initIDs()`'s `GetFieldID(Pointer, "peer", "J")` then throws
# `UnsatisfiedLinkError: Can't obtain peer field ID`, JNA's static initializer
# never recovers, and every later `Structure` allocation throws
# `NoClassDefFoundError: com.sun.jna.Native` — i.e. the whole FFI layer, so no
# endpoint can bind at all.

# JNA itself: fields are read by name from native code, so keep members too —
# keeping only the class names is not enough.
-keep class com.sun.jna.** { *; }

# JNA reads Structure subclasses field-by-field (name and declaration order), so
# their members must survive. Covers the generated `app.azula.iroh.*Struct`
# types, e.g. RustBufferStruct.
-keepclassmembers class * extends com.sun.jna.Structure { *; }

# `Library` method names ARE the native symbol names JNA dlsym's, and `Callback`
# methods are invoked from native (UniFFI's async continuations land here).
# Renaming either breaks the lookup.
-keep class * implements com.sun.jna.Library { *; }
-keep class * implements com.sun.jna.Callback { *; }

# The generated binding surface as a whole: every type here is either a JNA
# Structure, a Library/Callback, or reached from one. This package is a thin FFI
# shim, so there is nothing meaningful for R8 to shrink and the blanket keep
# removes a whole class of release-only breakage.
-keep class app.azula.iroh.** { *; }

# JNA references desktop AWT types that do not exist on Android; they are never
# reached at runtime here.
-dontwarn java.awt.**
