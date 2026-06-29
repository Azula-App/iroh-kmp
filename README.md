# iroh-kmp

A minimal [iroh](https://iroh.computer) SDK for **Kotlin Multiplatform**, packaged
as `app.azula.iroh`. It wraps iroh in a small Rust + [UniFFI](https://mozilla.github.io/uniffi-rs/)
crate and generates JNI-backed KMP bindings with [Gobley](https://gobley.dev).

## Why this exists

The published `computer.iroh:iroh` Maven artifact is UniFFI-**over-JNA**, and its
async calls never complete on Android (`Endpoint.bind` hangs, `nodeId` stays null).
Gobley generates **JNI**-backed bindings, whose async (tokio) calls do complete on
Android. This SDK exposes only the surface the azula app needs.

## Layout

```
Cargo.toml                         # the Rust crate (lib name `iroh_kmp`)
src/commonMain/rust/               # lib.rs, endpoint.rs, stream.rs, error.rs, android_init.rs
src/androidMain/kotlin/            # IrohAndroid.installAndroidContext (ndk_context init)
build.gradle.kts                   # KMP + Gobley (cargo + uniffi) + maven-publish
```

The UniFFI bindings (`IrohEndpoint`, `IrohStream`, `IncomingConn`, `IrohError`)
are generated at build time into the `app.azula.iroh` package.

## API

- `IrohEndpoint.bind(alpns, secretKey?)` → endpoint (reuses the key for a stable id)
- `endpoint.nodeId()`, `endpoint.secretKeyBytes()`
- `endpoint.myTicket()` — online + shareable ticket string
- `endpoint.connect(ticket, alpn)` → `IrohStream`
- `endpoint.acceptNext()` → `IncomingConn?` (loop into a Flow)
- `IrohStream.sendBytes(...)`, `recv(max)`, `close()`

On Android, call `IrohAndroid.installAndroidContext(applicationContext)` once at
startup before binding (iroh's DNS resolver needs the JavaVM + app context).

## Build & publish locally

Requires the Android NDK (r28+), Rust with the Android/iOS targets, and JDK 17.

```bash
# JDK 17 is required (AGP 8.7.x). Example:
export JAVA_HOME=/Library/Java/JavaVirtualMachines/zulu-17.54.21/Contents/Home
./gradlew publishToMavenLocal
```

Produces `app.azula.iroh:iroh-kmp:0.1.0` (+ per-target `-android`, `-jvm`,
`-iosarm64`, …) under `~/.m2/repository/app/azula/iroh/`.

## Toolchain versions

Kotlin 2.1.10 · AGP 8.7.3 · Gobley 0.3.7 · UniFFI 0.29 · Gradle 8.13 · iroh 1.0.
