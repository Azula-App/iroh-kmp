import gobley.gradle.GobleyHost
import gobley.gradle.cargo.dsl.jvm
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.atomicfu)
    alias(libs.plugins.gobley.cargo)
    alias(libs.plugins.gobley.uniffi)
    `maven-publish`
}

group = "app.azula.iroh"
version = "0.1.1"

// The Cargo.toml at the project root is picked up automatically by the cargo
// plugin; it cross-compiles the crate for every Kotlin target declared below.

cargo {
    // For the JVM target, Gobley otherwise tries to cross-build the desktop
    // library for every host OS (linux/windows/macos). We only need — and can
    // only link — the current host, so build the JVM lib for this machine only.
    builds.jvm {
        embedRustLibrary = (rustTarget == GobleyHost.current.rustTarget)
    }
}

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
// to the demo transport. Stage the dylib into the main jvm jar at the JNA
// resource path (`<jna-platform>/libiroh_kmp.dylib`) so any consumer loads it —
// mirroring how the Android AAR bundles its .so. (Host: macOS arm64.)
val jvmNativeResDir = layout.buildDirectory.dir("jvmNativeResources")
val embedJvmNativeLib = tasks.register<Copy>("embedJvmNativeLib") {
    val triple = "aarch64-apple-darwin"
    duplicatesStrategy = DuplicatesStrategy.INCLUDE
    from(layout.projectDirectory.dir("target/$triple/debug")) { include("libiroh_kmp.dylib") }
    from(layout.projectDirectory.dir("target/$triple/release")) { include("libiroh_kmp.dylib") }
    into(jvmNativeResDir.map { it.dir("darwin-aarch64") })
    dependsOn(tasks.matching { it.name.startsWith("cargoBuild") && it.name.contains("MacOSArm64") })
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
