import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

android {
    namespace = "com.holyblocker.mobile"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.holyblocker.mobile"
        // TYPE_ACCESSIBILITY_OVERLAY needs API 22; 26 is the floor for the
        // foreground-service and adaptive behaviour the daemon will want next.
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            // Must match the ABIs scripts/build-ffi.sh builds. The JNA aar ships
            // dispatchers for dead ABIs too (mips, armeabi, x86); without this
            // filter they land in the APK with no libtext_policy_ffi.so beside
            // them, so JNA would load and then fail to find the engine.
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    // Targets 17 bytecode while building on whatever JDK Gradle runs (21 here);
    // no toolchain pin, so the build does not need a second JDK provisioned.
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets["main"].java.srcDirs("src/main/kotlin", "src/generated/kotlin")
    sourceSets["test"].java.srcDirs("src/test/kotlin")
}

// The generated bindings are gitignored (they are build output of
// packages/text-policy-ffi), so a fresh clone has none. Without this check the
// failure is an unresolved-reference error pointing at NativeTextPolicy.kt,
// which says nothing about the actual cause.
val checkFfiBindings by tasks.registering {
    val bindings = file("src/generated/kotlin/uniffi")
    doLast {
        if (!bindings.exists()) {
            throw GradleException(
                """
                UniFFI bindings are missing: $bindings

                Generate them (needs cargo; the .so additionally needs the NDK):
                    ./scripts/build-ffi.sh
                """.trimIndent(),
            )
        }
    }
}

tasks.named("preBuild") { dependsOn(checkFfiBindings) }

dependencies {
    // Required by the UniFFI-generated bindings, which call into
    // libtext_policy_ffi.so through JNA. The @aar form bundles the native JNA
    // dispatcher for Android ABIs.
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    testImplementation("junit:junit:4.13.2")
}
