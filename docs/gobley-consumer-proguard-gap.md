# Gobley: generated ProGuard rules never reach consumers

**Status:** upstream bug, root cause found and fixed on the branch
`salvatoret/fix/uniffi-consumer-proguard-rules` in the Gobley checkout; not yet
submitted upstream. Worked around locally by `consumer-rules.pro` in this repo.
**Found:** 2026-07-20, against **Gobley 0.3.7**. **Diagnosed:** 2026-08-01.
**Upstream fix owner:** Sal (Gobley maintainer).

## Summary

Gobley generates exactly the right JNA ProGuard rules, has them enabled by
default, and wires them into the library's `consumerProguardFiles` — but the
wiring is silently dropped before it takes effect **whenever the Android Gradle
Plugin is applied before the Gobley plugins**, which is what this repo does. Any
such library gets an AAR with no `proguard.txt` at all, so every consuming app's
R8 run has a completely broken FFI layer, with no build-time signal.

## What Gobley actually does

`GenerateUniffiProguardRulesTask` emits JNA's own recommended Android rules
(copied from JNA's FAQ):

```
-dontwarn java.awt.*
-keep class com.sun.jna.* { *; }
-keepclassmembers class * extends com.sun.jna.* { public *; }
```

`UniFfiExtension.generateProguardRules` defaults to `convention(true)`, so this is
on out of the box. `UniFfiPlugin` resolves `androidGeneratedProguardFile` and
hands it to `GobleyAndroidExtensionDelegate.addProguardFiles`, whose impl
(`GobleyAndroidExtensionDelegateImpl.addProguardFilesToBuildType`) attaches it to
`buildType.proguardFile` for applications **and `buildType.consumerProguardFile`
for libraries** — the latter since 0.3.0 ([#140]). So the intent has always been
correct.

> **Correction.** An earlier revision of this note claimed Gobley never
> referenced consumer ProGuard files at all, based on
> `grep -rla "consumerProguardFiles" <extracted gobley-gradle*.jar>` returning
> nothing. That was a false negative: the code calls the **singular**
> `consumerProguardFile`. The conclusion drawn from it was wrong.

## The actual root cause

An ordering bug in `addProguardFilesToBuildType`: the DSL mutation is interleaved
with calls to `project.tasks.withType<T> { ... }`.

1. The Kotlin DSL `withType` overload that takes an action is the **eager**
   `DomainObjectCollection.withType(Class, Action)` — it realizes every matching
   task immediately.
2. Realizing `AndroidLintAnalysisTask` (via `VariantInputs.initialize`) reads
   `OptimizationCreationConfig.consumerProguardFiles`. That property is backed by
   a `kotlin.Lazy` whose initializer calls
   `OptimizationDslInfo.gatherProguardFiles(CONSUMER)` and **freezes the gathered
   list into an immutable `List<File>`**.
3. `buildType.consumerProguardFile(proguardFile)` was called *after* those
   `withType` blocks. By then the list is frozen, so the add mutates something
   nothing reads again.
4. Worse, the enclosing `buildTypes.configureEach` iterates debug then release,
   but each `withType` realizes tasks for *all* variants — so the **debug**
   iteration freezes **release's** list before release is ever mutated. The
   published release AAR loses the rules essentially unconditionally.

### Why upstream CI never caught it

The bug only fires if the variant tasks already exist when `addProguardFiles`
runs — i.e. when `com.android.library` is applied **before** the Gobley plugins,
so AGP's `afterEvaluate` has already registered them. This repo's `plugins {}`
block does exactly that.

Every test and example in the Gobley repo applies `com.android.library` **last**,
after the Gobley plugins. There the tasks do not exist yet, the eager `withType`
calls find nothing to realize, nothing is frozen, and the rules land correctly.
Confirmed empirically: `:tests:uniffi:coverall-android` ships the generated block
in its AAR on `main`; reordering its `plugins {}` block to apply AGP first makes
`proguard.txt` disappear entirely.

Note also that AGP is *not* at fault on either axis originally suspected: it does
honor build-type-level `consumerProguardFiles` for libraries
(`gatherProguardFiles` reads `postProcessingOptions`, which defaults to the build
type), and its task registration is lazy, so Gobley running in a later
`afterEvaluate` would have been fine on its own.

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

1. Take any Gobley UniFFI library with an `androidTarget`, applying
   `com.android.library` **before** the Gobley plugins, and publish the AAR.
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

## The upstream fix

Mutate every build type **first**, then wire the task dependencies through the
lazy `withType<T>().configureEach { }` so nothing is force-realized mid-way. The
per-task name filters can go: the same generation task was added once per build
type, so the union over all build types is exactly "every such task depends on
it".

Verified against `:tests:uniffi:coverall-android` with its `plugins {}` block
temporarily reordered to reproduce the affected configuration — `proguard.txt`
goes from absent to carrying the generated block, and the original plugin order
is unaffected.

Still open, and deliberately left to a separate change: the generated rule set is
narrower than what a UniFFI binding actually needs.

- `com.sun.jna.*` (single star) misses subpackages; `com.sun.jna.**` is safer.
- `-keepclassmembers class * extends com.sun.jna.* { public *; }` keeps only
  *public* members of `Structure` subclasses.
- Nothing keeps the **generated binding package**. JNA maps `Library` interface
  method names straight to `dlsym` symbol names, and `Callback` methods are
  invoked from native, so renaming either breaks lookup. Gobley knows the
  namespace (`bindingsGeneration.namespace`), so it can emit a targeted
  `-keep class <namespace>.** { *; }`.

Worth flagging upstream too: because every Gobley test and example applies AGP
after the Gobley plugins, none of them covers the configuration this bug needs, so
CI would not catch a regression. Reordering one existing Android library test
project's `plugins {}` block would close that gap in one line.

## Local workaround (this repo)

`consumer-rules.pro` + `consumerProguardFiles("consumer-rules.pro")` in
`build.gradle.kts`, carrying the widened rule set above. Verified: the consuming
release build leaves `Pointer.peer` unrenamed, the app binds an endpoint and
reports online, and the app's *own* fields are still obfuscated — so the rules
are correctly scoped rather than disabling minification.

This works because `defaultConfig` is populated during script evaluation, long
before anything can freeze the gathered list.

Once the upstream fix lands and this repo moves to a Gobley version carrying it,
`consumer-rules.pro` can shrink to just whatever Gobley still doesn't cover (or
be dropped entirely, if the widened rules land too).

[#140]: https://github.com/gobley/gobley/pull/140
