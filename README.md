# iroh-kmp

A full-featured [iroh](https://iroh.computer) SDK for **Kotlin Multiplatform**,
packaged as `app.azula.iroh`. It wraps the `iroh` 1.0 Rust crate in a small +
[UniFFI](https://mozilla.github.io/uniffi-rs/) crate and generates JNI-backed KMP
bindings with [Gobley](https://gobley.dev). Targets: `jvm`, `android`, and Apple
(`iosArm64`, `iosSimulatorArm64`, `iosX64`).

## Why this exists

The published `computer.iroh:iroh` Maven artifact is UniFFI-**over-JNA**, and its
async calls never complete on Android (`Endpoint.bind` hangs, `id` stays null).
Gobley generates **JNI**-backed bindings, whose async (tokio) calls do complete on
Android. iroh-kmp started as a minimal transport for the azula app and now exposes
the core iroh API so it's usable as a standalone SDK.

## Install (Maven Central)

```kotlin
// build.gradle.kts / KMP dependencies
implementation("app.azula.iroh:iroh-kmp:0.1.0")
```

The bindings are generated at build time into the `app.azula.iroh` package. On
**Android**, call `IrohAndroid.installAndroidContext(applicationContext)` once at
startup **before** binding any endpoint (iroh's DNS resolver needs the JavaVM +
app context). The AAR bundles the per-ABI native libs, so no manual `jniLibs` are
needed.

## API

Everything is generated into `app.azula.iroh`. The pre-1.0 transport surface
(`connect`/`accept_next`/`IrohStream`/tickets) is unchanged and kept as a
convenience layer over the richer `IrohConnection` API.

### Endpoint — bind & identity
- `IrohEndpoint.bind(alpns, secretKey?)` — the common case: n0 production relays +
  discovery, reusing `secretKey` (32 bytes) for a stable node id.
- `IrohEndpoint.bind_with(EndpointOptions)` — full control: `relayMode`
  (`Default` / `Disabled` / `Custom(urls)`), `addressLookup` on/off, `bindAddr`,
  `externalAddrs`, `warmUpOnline`.
- `id()`, `secretKeyBytes()`, `sign(data)`, `isClosed()`, `shutdown()`
- `setAlpns(alpns)`, `networkChange()`

### Endpoint — dial
- `connect(ticket, alpn)` → `IrohStream` (ticket + one bi-stream; the app's path)
- `connectConn(ticket, alpn)` → `IrohConnection`
- `connectAddr(EndpointAddr, alpn)` → `IrohConnection`
- `connectById(endpointIdHex, alpn)` → `IrohConnection` (resolves via address lookup)

### Endpoint — accept
- `acceptNext()` → `IncomingConn?` (ticket-era convenience: connection + accepted bi-stream)
- `acceptConn()` → `IrohConnection?`
- `acceptNext` and `acceptConn` share one single-consumer accept queue — pick one loop.

### Connection (`IrohConnection`)
- streams: `openBi()`/`acceptBi()` → `IrohStream`; `openUni()` → `IrohSendStream`;
  `acceptUni()` → `IrohRecvStream`
- datagrams: `sendDatagram(data)` (waits for buffer space), `trySendDatagram(data)`
  (fails fast), `readDatagram()`, `maxDatagramSize()`
- lifecycle: `shutdown(errorCode, reason)` (named `shutdown`, not `close`),
  `closed()`, `closeReason()`
- info: `remoteNodeId()`, `alpn()`, `stableId()`, `rttMs()`, `paths()` (`ConnPath`
  list), `connType()` (`"direct"`/`"relay"`/`"mixed"`/`"none"`)

### Streams
- `IrohStream` (bidirectional): `sendBytes(...)`, `recv(max)`, `readExact(n)`,
  `readToEnd(max)`, `finish()`, `rttMs()`, `setPriority`/`priority`, `reset(code)`,
  `stop(code)`, `sendId()`/`recvId()`
- `IrohSendStream` (uni): `sendBytes`, `finish`, `reset`, `setPriority`/`priority`,
  `stopped()`, `id()`
- `IrohRecvStream` (uni): `recv(max)`, `readExact(n)`, `readToEnd(max)`, `stop`, `id()`

`finish` is named to avoid colliding with UniFFI's generated `AutoCloseable.close()`.

### Addresses, status & remote info
- `myTicket()` — online + a shareable `EndpointTicket` string ("code")
- `addr()` / `addrUpdated()` — current addressing snapshot / await next change
- `directAddresses()`, `homeRelay()`, `boundSockets()`, `waitOnline()`
- `remoteInfo(endpointIdHex)` → `RemoteInfo?` (per-address `RemoteAddrInfo`)
- free fns: `endpointIdFromTicket`, `verifySignature`, `ticketBytes`, `ticketFromBytes`,
  `endpointAddrFromTicket`, `ticketFromEndpointAddr`

Watchers are surfaced as snapshot + `…Updated()` accessors (UniFFI can't ship a
`Watcher`/stream across the FFI); loop `addrUpdated()` into a Kotlin `Flow` the
same way you loop `acceptNext()`.

> Metrics are not yet exposed (see the `// TODO` in `endpoint.rs`).

## Build & publish locally

Requires the Android NDK (r28+), Rust with the Android/iOS targets, and JDK 17
(AGP 8.7.x).

```bash
export JAVA_HOME=/Library/Java/JavaVirtualMachines/zulu-17.54.21/Contents/Home
./gradlew publishToMavenLocal    # → ~/.m2/repository/app/azula/iroh/iroh-kmp/<version>/
```

`publishToMavenLocal` works without GPG (signing is applied only when a signing key
is present). Fast Rust feedback loop: `cargo build` / `cargo test` / `cargo clippy`.
Generate API docs with `./gradlew dokkaGenerate` (HTML in `build/dokka/html`).

## Releasing to Maven Central

Publishing is automated via the
[vanniktech maven-publish](https://vanniktech.github.io/gradle-maven-publish-plugin/)
plugin → the Central Portal, driven by `.github/workflows/publish.yml` on a `v*`
tag. To cut a release:

1. Bump `VERSION_NAME` in `gradle.properties` **and** `version` in `Cargo.toml`.
2. Commit, then push a matching tag: `git tag v0.1.0 && git push origin v0.1.0`.
3. The workflow (on `macos-latest`, so it can build every KMP target in one
   publication set) signs and uploads all artifacts.

**Prerequisites** (one-time, out of band — the workflow is inert until these exist):
verify the `app.azula` namespace on the Central Portal, and add repo secrets
`MAVEN_CENTRAL_USERNAME`, `MAVEN_CENTRAL_PASSWORD`, `SIGNING_IN_MEMORY_KEY`,
`SIGNING_IN_MEMORY_KEY_PASSWORD` (mapped in CI to the `ORG_GRADLE_PROJECT_*`
properties vanniktech reads). See `azula-docs/openspec/specs/iroh-kmp/design.md` for the full
runbook. For the very first release, swap `publishAndReleaseToMavenCentral` for
`publishToMavenCentral` to review the staged deployment in the Portal before
releasing.

## Docs site

`.github/workflows/docs.yml` builds the Dokka HTML on push to `main` and uploads it
as the `dokka-html` artifact. The GitHub Pages deploy job is wired but gated behind
`repository.private` (Pages on a private repo needs Enterprise); it activates
automatically once the repo is public and Pages is enabled (Source: GitHub Actions).

## Toolchain versions

Kotlin 2.1.10 · AGP 8.7.3 · Gobley 0.3.7 · UniFFI 0.29 · Gradle 8.13 · iroh 1.0 ·
Dokka 2.2.0 · vanniktech maven-publish 0.35.0.

## License

Dual-licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
