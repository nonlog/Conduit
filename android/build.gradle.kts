buildscript {
    repositories { google(); mavenCentral() }
    dependencies {
        // AGP 9 uses built-in Kotlin; keep its compiler aligned with Compose.
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.10")
    }
}
plugins {
    id("com.android.application") version "9.1.1" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.10" apply false
    id("com.google.protobuf") version "0.10.0" apply false
}
