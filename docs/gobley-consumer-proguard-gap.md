# Gobley: generated ProGuard rules never reach consumers

**Status:** upstream bug, worked around locally by `consumer-rules.pro` in this repo.
**Found:** 2026-07-20, against **Gobley 0.3.7**.
**Upstream fix owner:** Sal (Gobley maintainer) — this note exists so the context
survives until it's filed.

## Summary

Gobley generates exactly the right JNA ProGuard rules, has them enabled by
default, and then wires them somewhere that **no consumer of the published
library ever sees**. Any app that consumes a Gobley-generated KMP library and
runs R8 (i.e. any Android release build) gets a completely broken FFI layer, with
no build-time signal.

## What Gobley does today

`GenerateUniffiProguardRulesTask` emits JNA's own recommended Android rules
(copied from JNA's FAQ):

```
-keep class com.sun.jna.* { *; }
-keepclassmembers class * extends com.sun.jna.* { public *; }
-dontwarn java.awt.*
```

`UniFfiExtension.generateProguardRules` defaults to `convention(true)`, so this is
on out of the box. `UniFfiPlugin` resolves `androidGeneratedProguardFile` and hands
it to `GobleyAndroidExtensionDelegate.addProguardFiles`, whose impl
(`GobleyAndroidExtensionDelegateImpl.addProguardFilesToBuildType`) attaches it to
`com.android.build.api.dsl.BuildType` — i.e. **`buildType.proguardFiles`**.

That is the *library's own* minification config. For a `com.android.library`
project it does nothing for downstream apps. Confirmed by decompiling the 0.3.7
plugin jars:

```
$ grep -rla "consumerProguardFiles" <extracted gobley-gradle*.jar>
  # (no output — no Gobley class references consumerProguardFiles at all)
```

## Why it matters

JNA resolves members from native code by their **literal names**. R8 renames
`com.sun.jna.Pointer.peer` → `d`; `Native.initIDs()` then fails its
`GetFieldID(Pointer, "peer", "J")`, throws `UnsatisfiedLinkError`, and JNA's
static initializer never recovers — so every later `Structure` allocation throws
`NoClassDefFoundError: com.sun.jna.Native`. Since UniFFI allocates a
`RustBufferStruct` on essentially every call, **the entire FFI layer is dead** in
release builds.

Note R8's default rules *do* keep `native` method names, which is why this
presents as a field problem specifically. Nothing protects fields by default.

Real-world impact here: azula's Android release builds could not bind an iroh
endpoint at all. It went unnoticed for two store releases because the app caught
its startup bind failure and degraded to "offline"; the crash only surfaced on a
later code path that called the FFI without a guard.

## Minimal replication

Static check — no device, no app code needed:

1. Take any Gobley UniFFI library with an `androidTarget`, publish the AAR.
2. Confirm the rules did **not** ship with it:
   ```bash
   unzip -p <lib>-android-<ver>.aar proguard.txt   # absent
   ```
3. Consume it from an Android app with R8 on (release), build, then inspect the
   consumer's `mapping.txt`:
   ```bash
   grep -A 6 "^com.sun.jna.Pointer ->" .../outputs/mapping/release/mapping.txt
   ```
   Broken looks like:
   ```
   com.sun.jna.Pointer -> com.sun.jna.Pointer:
       long peer -> d          # ← renamed; FFI will die at runtime
   ```
   Fixed looks like: the same class block with **no `peer` line at all** (the
   field survived unrenamed).

Runtime confirmation, if wanted: call any bindings function and watch for
`NoClassDefFoundError: com.sun.jna.Native` caused by
`UnsatisfiedLinkError: Can't obtain peer field ID for class com.sun.jna.Pointer`.

Note step 3 is the useful assertion for CI — it catches the regression without a
device and without needing a crash to reproduce.

## Proposed upstream fix

For `com.android.library` projects, also add the generated file to
`defaultConfig.consumerProguardFiles` so the rules propagate into the AAR (they
end up as `proguard.txt`) and get applied by every consumer's R8 run. Keep the
existing `buildType.proguardFiles` wiring for application projects.

While in there, the generated rule set is worth widening — it is narrower than
what a UniFFI binding actually needs:

- `com.sun.jna.*` (single star) misses subpackages; `com.sun.jna.**` is safer.
- `-keepclassmembers class * extends com.sun.jna.* { public *; }` keeps only
  *public* members of `Structure` subclasses.
- Nothing keeps the **generated binding package**. JNA maps `Library` interface
  method names straight to `dlsym` symbol names, and `Callback` methods are
  invoked from native, so renaming either breaks lookup. Gobley knows the
  namespace (`bindingsGeneration.namespace`), so it can emit a targeted
  `-keep class <namespace>.** { *; }`.

## Local workaround (this repo)

`consumer-rules.pro` + `consumerProguardFiles("consumer-rules.pro")` in
`build.gradle.kts`, carrying the widened rule set above. Verified: the consuming
release build leaves `Pointer.peer` unrenamed, the app binds an endpoint and
reports online, and the app's *own* fields are still obfuscated — so the rules
are correctly scoped rather than disabling minification.

Once the upstream fix lands and this repo moves to a Gobley version carrying it,
`consumer-rules.pro` can shrink to just whatever Gobley still doesn't cover (or
be dropped entirely, if the widened rules land too).
