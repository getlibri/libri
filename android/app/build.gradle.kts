// The Libri app: Compose over the Rust core (../../client), which Gradle
// builds for each phone architecture with cargo-ndk and binds with UniFFI.
// Run inside `nix develop .#android`, which supplies the SDK, NDK, JDK and
// Rust.
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Libri has one version, the server crate's, so the web and the phone say
// the same number for the same release; ./set_version.sh bumps it.
val libriVersion = Regex("""(?m)^version = "([^"]+)"""")
    .find(providers.fileContents(layout.projectDirectory.file("../../libri/Cargo.toml")).asText.get())!!
    .groupValues[1]
// Android wants a rising integer: 0.9.17 is 9017.
val libriVersionCode = libriVersion.split(".").map(String::toInt)
    .let { (major, minor, patch) -> major * 1_000_000 + minor * 1_000 + patch }

val core = rootProject.file("../client")
val coreLibs = layout.buildDirectory.dir("core/jniLibs")
val coreKotlin = layout.buildDirectory.dir("core/kotlin")

android {
    // Kotlin reserves `is`, so the code lives in su.libri.
    namespace = "su.libri"
    compileSdk = 37
    // Kept in step with nix/android.nix, which supplies the SDK read-only.
    ndkVersion = "29.0.14206865"
    buildToolsVersion = "37.0.0"

    defaultConfig {
        // Permanent once installed: a phone treats another id as another app.
        // 0.9.23 moved it from is.su.libri to li.libri, and the next release to is.libri
        // (libri.is is the product domain); each installs beside the one before.
        applicationId = "is.libri"
        minSdk = 29
        targetSdk = 37
        versionCode = libriVersionCode
        versionName = libriVersion
        // Phones (arm64) and the emulator (x86_64): the core is built for no other.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    // The release key stays off the repository; the build reads where it
    // is from the environment (LIBRI_KEYSTORE, LIBRI_KEYSTORE_PASSWORD).
    signingConfigs {
        create("release") {
            System.getenv("LIBRI_KEYSTORE")?.let {
                storeFile = file(it)
                storePassword = System.getenv("LIBRI_KEYSTORE_PASSWORD")
                keyAlias = "libri"
                keyPassword = System.getenv("LIBRI_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (System.getenv("LIBRI_KEYSTORE") != null) signingConfig = signingConfigs.getByName("release")
        }
    }

    buildFeatures { compose = true }

    sourceSets["main"].jniLibs.srcDir(coreLibs.get().asFile)
    sourceSets["main"].kotlin.srcDir(coreKotlin.get().asFile)

    packaging { jniLibs.useLegacyPackaging = false }
}

// The core for the phones (arm64) and the emulator (x86_64), then its
// Kotlin bindings, read from the library itself.
val buildCore by tasks.registering(Exec::class) {
    workingDir = core
    inputs.dir(core.resolve("src"))
    inputs.files(core.resolve("Cargo.toml"), core.resolve("Cargo.lock"), core.resolve("uniffi.toml"))
    outputs.dirs(coreLibs, coreKotlin)
    val libs = coreLibs.get().asFile
    val kotlin = coreKotlin.get().asFile
    commandLine(
        "sh", "-ec",
        """
        cargo ndk --platform 29 -t arm64-v8a -t x86_64 -o '$libs' build --release
        rm -rf '$kotlin'
        cargo run --quiet --release --features bindgen --bin uniffi-bindgen -- \
          generate --library target/aarch64-linux-android/release/liblibri_client.so \
          --language kotlin --no-format --out-dir '$kotlin'
        """.trimIndent(),
    )
}
tasks.named("preBuild") { dependsOn(buildCore) }

dependencies {
    val compose = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(compose)
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.core:core-ktx:1.19.1")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")
    // Replies are GitHub-flavoured markdown, as the web page renders them.
    implementation("com.mikepenz:multiplatform-markdown-renderer-m3:0.46.0")
    implementation("com.mikepenz:multiplatform-markdown-renderer-coil3:0.46.0")
    // Pictures from the host (/api/file); OkHttp also carries the terminal's socket.
    implementation("io.coil-kt.coil3:coil-compose:3.6.3")
    implementation("io.coil-kt.coil3:coil-network-okhttp:3.6.3")
    implementation("com.squareup.okhttp3:okhttp:5.5.1")
    // Serves the terminal's page from the APK's assets at an https origin.
    implementation("androidx.webkit:webkit:1.18.0")
    // Notifications by the phone's UnifiedPush distributor (ntfy), no Google services.
    implementation("org.unifiedpush.android:connector:3.3.5")
    // UniFFI's Kotlin calls the core through JNA.
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
