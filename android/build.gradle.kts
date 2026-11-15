plugins {
    id("com.android.application") version "9.5.0" apply false
    // The Android Gradle plugin compiles Kotlin itself; this names the
    // Kotlin it uses, which the Compose compiler must match.
    id("org.jetbrains.kotlin.android") version "2.4.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.21" apply false
}
