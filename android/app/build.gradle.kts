plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.google.protobuf")
}

android {
    namespace = "com.conduit.sync"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.conduit.sync"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
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

    // One screen, Material 3 with dynamic colour. Compose is not resident: nothing
    // here is loaded while only the sync service is running.
    // BOM pinned below 2026.08.00, whose Compose 1.12 demands AGP 9.1 and compileSdk 37;
    // AGP cannot move while Gradle is pinned at 8.14.3. Bump both together or not at all.
    implementation(platform("androidx.compose:compose-bom:2026.06.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.13.0")

    // Reading the clipboard from the background is impossible on stock Android 10+, so
    // ClipboardHook lifts the check inside system_server. compileOnly: LSPosed supplies
    // the implementation, and nothing from this jar may end up in the APK.
    compileOnly("de.robv.android.xposed:api:82")

    // Noise XX is hand-written, so it is checked against a transcript produced by the
    // Rust reference implementation. Plain JVM test — no device, no instrumentation.
    testImplementation("junit:junit:4.13.2")
}
