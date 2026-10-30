plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.google.protobuf")
}

android {
    namespace = "com.conduit.sync"
    compileSdk { version = release(37) { minorApiLevel = 0 } }

    defaultConfig {
        applicationId = "com.conduit.sync"
        minSdk = 29
        targetSdk = 36
        versionCode = 15
        versionName = "0.1.14"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildFeatures {
        compose = true
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The wire contract lives once, at the repo root, and both sides generate from it.
    // protobuf-gradle-plugin 0.10.0 only ships a Kotlin DSL accessor for JVM SourceSet,
    // not for AGP's source sets, so the extension is fetched by name.
    sourceSets {
        getByName("main") {
            val proto = (this as ExtensionAware).extensions
                .getByName("proto") as org.gradle.api.file.SourceDirectorySet
            proto.srcDir("../../proto")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

protobuf {
    protoc { artifact = "com.google.protobuf:protoc:4.36.0" }
    generateProtoTasks {
        all().forEach { task ->
            task.builtins {
                // Android tasks start with no builtin at all, hence create not configure.
                // Lite runtime: the full one is ~1 MB of reflection we never use.
                maybeCreate("java").option("lite")
            }
        }
    }
}

dependencies {
    implementation("com.google.protobuf:protobuf-javalite:4.36.0")
    // X25519, ChaCha20-Poly1305 and BLAKE2s. Noise XX is built on these because no
    // Noise library is published for Java, and minSdk 29 predates platform XDH.
    implementation("org.bouncycastle:bcprov-jdk18on:1.85.2")

    // Official public Material 3 Expressive theme and component APIs.
    // The beta is explicitly pinned; the remaining Compose libraries use the stable BOM.
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.material3:material3:1.5.0-beta01")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.13.0")
    // Code Scanner brings an obsolete Fragment transitively; align ActivityResult support.
    implementation("androidx.fragment:fragment:1.9.1")

    // On-demand QR pairing. Google Code Scanner delegates camera access and image processing to
    // Play services, so Conduit itself needs no CAMERA permission and no resident camera pipeline.
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")

    // Reading the clipboard from the background is impossible on stock Android 10+, so
    // ClipboardHook lifts the check inside system_server. compileOnly: LSPosed supplies
    // the implementation, and nothing from this jar may end up in the APK.
    compileOnly("de.robv.android.xposed:api:82")

    // Noise XX is hand-written, so it is checked against a transcript produced by the
    // Rust reference implementation. Plain JVM test — no device, no instrumentation.
    testImplementation("junit:junit:4.13.2")

    // Instrumented Compose layout checks run only on GitHub Actions emulator. They never
    // touch an actual phone's pairing, notification permissions or persisted state.
    // androidTest has its own Gradle dependency configuration, so the main source-set
    // Compose BOM does not supply test-module versions here.
    androidTestImplementation(platform("androidx.compose:compose-bom:2026.09.00"))
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}
