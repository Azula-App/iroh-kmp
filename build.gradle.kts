import com.vanniktech.maven.publish.JavadocJar
import com.vanniktech.maven.publish.KotlinMultiplatform
import gobley.gradle.GobleyHost
import gobley.gradle.Variant
import gobley.gradle.cargo.dsl.jvm
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.atomicfu)
    alias(libs.plugins.gobley.cargo)
    alias(libs.plugins.gobley.uniffi)
    alias(libs.plugins.vanniktech.maven.publish)
    alias(libs.plugins.dokka)
}

// Single source of truth for coordinates, shared with Gobley's cargo/uniffi
// tasks and vanniktech's mavenPublishing block below. CI overrides
// VERSION_NAME from the release tag via -P.
group = providers.gradleProperty("GROUP").get()
version = providers.gradleProperty("VERSION_NAME").get()

// The Cargo.toml at the project root is picked up automatically by the cargo
// plugin; it cross-compiles the crate for every Kotlin target declared below.

cargo {
    // Build native (Kotlin/Native, i.e. the iOS cinterop) targets in release.
    // Gobley's `nativeVariant` defaults to Debug, which embeds the ~375 MB debug
    // static lib into each iOS klib (~99 MB published). Release drops the static
    // lib to ~19 MB per arch. JVM already publishes release (`jvmPublishingVariant`
    // defaults to Release) and Android maps `publishLibraryVariants("release")` to
    // the release .so, so only the native variant needs pinning. This lives here
    // (not in a workflow) so every publish — local and CI — uses release.
    nativeVariant = Variant.Release

    // For the JVM target, Gobley otherwise tries to cross-build the desktop
    // library for every host OS (linux/windows/macos). We only need — and can
    // only link — the current host, so build the JVM lib for this machine only.
    builds.jvm {
        embedRustLibrary = (rustTarget == GobleyHost.current.rustTarget)
    }
}

// --- Don't cross-compile Linux/Windows JVM variants we never publish --------
// embedRustLibrary above only controls which target's output gets embedded in
// the jvm jar — Gobley still wires cargoBuildLinux*/cargoBuildMinGW* into the
// task graph for `publishToMavenLocal` (they'd back per-host "rust-runtime"
// classifier jars), so they still run and fail on a Mac (no
// aarch64-linux-gnu-gcc cross-linker; and even where a mingw-w64 toolchain
// happens to be on PATH for unrelated reasons, this crate's huge dependency
// tree hits mingw's PE export-table limit — "export ordinal too large").
// Only the Apple + Android targets are actually published/consumed
// (azula-app depends on app.azula.iroh:iroh-kmp for those), so just disable
// the Linux/Windows cargo builds outside of their native host — the same
// idea as the iOS `GobleyHost.Platform.MacOS.isCurrent` gate below — instead
// of requiring
// `-x cargoBuildLinux… -x cargoBuildMinGW…` by hand on every other host.
tasks.matching { it.name.startsWith("cargoBuildLinux") }
    .configureEach { enabled = GobleyHost.Platform.Linux.isCurrent }
tasks.matching { it.name.startsWith("cargoBuildMinGW") }
    .configureEach { enabled = GobleyHost.Platform.Windows.isCurrent }

uniffi {
    generateFromLibrary {
        // The package the generated bindings (IrohEndpoint, IrohStream, ...) land in.
        packageName = "app.azula.iroh"
    }
}

// --- Embed the host JVM native lib into the MAIN jvm jar --------------------
// Gobley publishes the JVM dylib only in a per-host "rust-runtime" *classifier*
// jar, and its module metadata doesn't list that jar in the runtime variant — so
// a plain-metadata consumer like Amper never puts it on the classpath, the JNA
// binding fails (UnsatisfiedLinkError), and the desktop app silently falls back
// to the demo transport. Stage the dylib/so/dll into the main jvm jar at the JNA
// resource path (`<jna-platform>/lib*`) so any consumer loads it — mirroring how
// the Android AAR bundles its .so.
//
// The host triple/JNA-platform-dir/lib-filename below are derived from
// os.name/os.arch rather than hardcoded, so this works on any host — a stale
// hardcoded triple would silently ship an *empty* jvmNativeResources dir on any
// other host, and the failure would only surface much later as an
// UnsatisfiedLinkError at runtime.
data class HostNativeLib(val rustTriple: String, val jnaPlatformDir: String, val libFileName: String)

fun hostNativeLib(): HostNativeLib {
    val osName = System.getProperty("os.name", "").lowercase()
    val osArch = System.getProperty("os.arch", "").lowercase()
    val arch = when (osArch) {
        "aarch64", "arm64" -> "aarch64"
        "x86_64", "amd64" -> "x86_64"
        else -> error("embedJvmNativeLib: unsupported host arch '$osArch'")
    }
    return when {
        osName.contains("mac") || osName.contains("darwin") -> HostNativeLib(
            rustTriple = "$arch-apple-darwin",
            jnaPlatformDir = "darwin-${if (arch == "x86_64") "x86-64" else arch}",
            libFileName = "libiroh_kmp.dylib",
        )
        osName.contains("linux") -> HostNativeLib(
            rustTriple = "$arch-unknown-linux-gnu",
            jnaPlatformDir = "linux-${if (arch == "x86_64") "x86-64" else arch}",
            libFileName = "libiroh_kmp.so",
        )
        osName.contains("windows") -> HostNativeLib(
            rustTriple = "x86_64-pc-windows-msvc",
            jnaPlatformDir = "win32-x86-64",
            libFileName = "iroh_kmp.dll",
        )
        else -> error("embedJvmNativeLib: unsupported host OS '$osName'")
    }
}

val jvmNativeResDir = layout.buildDirectory.dir("jvmNativeResources")
val embedJvmNativeLib = tasks.register<Copy>("embedJvmNativeLib") {
    val native = hostNativeLib()
    duplicatesStrategy = DuplicatesStrategy.INCLUDE
    from(layout.projectDirectory.dir("target/${native.rustTriple}/debug")) { include(native.libFileName) }
    from(layout.projectDirectory.dir("target/${native.rustTriple}/release")) { include(native.libFileName) }
    into(jvmNativeResDir.map { it.dir(native.jnaPlatformDir) })
    dependsOn(tasks.matching { it.name.startsWith("cargoBuild") })

    // Fail loudly instead of silently shipping a jar with no native lib for this
    // host (which would only surface later as an UnsatisfiedLinkError).
    doLast {
        val copied = jvmNativeResDir.get().dir(native.jnaPlatformDir).file(native.libFileName).asFile
        check(copied.exists()) {
            "embedJvmNativeLib: expected native library not found at ${copied.absolutePath} " +
                "(host rust target ${native.rustTriple}). The jvm jar would silently ship without " +
                "it, causing UnsatisfiedLinkError at runtime instead of failing the build now. " +
                "Check that `cargo build` produced " +
                "target/${native.rustTriple}/{debug,release}/${native.libFileName}."
        }
    }
}
tasks.matching { it.name == "jvmProcessResources" }.configureEach { dependsOn(embedJvmNativeLib) }

kotlin {
    androidTarget {
        publishLibraryVariants("release")
        compilerOptions {
            jvmTarget = JvmTarget.JVM_17
        }
    }
    jvmToolchain(17)
    jvm()

    // iOS is only buildable on macOS; gate it so the project still configures on Linux/CI.
    if (GobleyHost.Platform.MacOS.isCurrent) {
        iosArm64()
        iosSimulatorArm64()
        iosX64()
    }

    sourceSets {
        commonMain.dependencies {
            implementation(libs.kotlinx.coroutines.core)
        }
        jvmMain {
            // Carry the host dylib inside the main jvm jar (see embedJvmNativeLib).
            resources.srcDir(jvmNativeResDir)
        }
    }
}

android {
    namespace = "app.azula.iroh"
    compileSdk = libs.versions.android.compileSdk.get().toInt()

    defaultConfig {
        minSdk = 26
        ndk.abiFilters.addAll(listOf("arm64-v8a", "armeabi-v7a", "x86_64"))
    }

    // r28+ for 16 KB page alignment (Android 15+).
    ndkVersion = "28.0.13004108"

    // Gobley's UniFFI bindings call through JNA, which on Android needs its native
    // dispatch lib. The plain `jna` jar Gobley pulls carries no Android .so, so we
    // bundle libjnidispatch.so (per ABI, from the jna aar) directly into our AAR —
    // this keeps the SDK self-contained: consumers need no manual jniLibs.
    sourceSets.getByName("main").jniLibs.srcDir("src/androidMain/jniLibs")

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(17)
    }
}

mavenPublishing {
    publishToMavenCentral()

    // Only sign when a key is actually configured, so publishToMavenLocal
    // keeps working for local testing without one.
    if (providers.gradleProperty("signingInMemoryKey").isPresent) {
        signAllPublications()
    }

    configure(
        KotlinMultiplatform(
            // An empty javadoc jar (Central only requires the jar to exist) keeps the
            // publish path independent of Dokka — Gobley's shared iOS binding dir trips
            // Dokka's validity check (see the `dokka {}` block). The browsable API docs
            // are served from the GitHub Pages site built by the docs workflow.
            javadocJar = JavadocJar.Empty(),
            sourcesJar = true,
            androidVariantsToPublish = listOf("release"),
        )
    )

    pom {
        name = "iroh-kmp"
        description = "Kotlin Multiplatform SDK for iroh peer-to-peer QUIC connections, " +
            "with UniFFI/Gobley bindings over the iroh Rust crate."
        url = "https://github.com/Azula-App/iroh-kmp"
        inceptionYear = "2025"

        licenses {
            license {
                name = "MIT License"
                url = "https://opensource.org/licenses/MIT"
            }
            license {
                name = "Apache License 2.0"
                url = "https://www.apache.org/licenses/LICENSE-2.0"
            }
        }

        developers {
            developer {
                id = "sal"
                name = "Sal"
                url = "https://github.com/Azula-App"
            }
        }

        scm {
            url = "https://github.com/Azula-App/iroh-kmp"
            connection = "scm:git:git://github.com/Azula-App/iroh-kmp.git"
            developerConnection = "scm:git:ssh://git@github.com/Azula-App/iroh-kmp.git"
        }
    }
}

dokka {
    moduleName = "iroh-kmp"

    dokkaPublications.html {
        outputDirectory = layout.buildDirectory.dir("dokka/html")
    }

    dokkaSourceSets.configureEach {
        sourceLink {
            remoteUrl("https://github.com/Azula-App/iroh-kmp/tree/main")
            localDirectory.set(rootDir)
        }

        // Gobley generates a single shared `nativeMain` binding dir for all three
        // iOS targets, so Dokka's pre-generation validity check rejects the source
        // root as shared across source sets (Kotlin/dokka#3701). The generated
        // `app.azula.iroh` API is identical across them, so document it once via
        // iosArm64 and suppress the duplicates. `startsWith` covers the DGP source-set
        // name whether or not it carries the `Main` suffix.
        if (name.startsWith("iosSimulatorArm64") || name.startsWith("iosX64")) {
            suppress.set(true)
        }
    }
}
