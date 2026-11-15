# The Android toolchain: the SDK and NDK from nixpkgs' androidenv, a JDK,
# Gradle, and a Rust that can build the client core for phones (nixpkgs'
# rustc carries no standard library for Android targets, so rust-overlay's
# does). `emulator = true` adds the emulator and an x86_64 system image,
# a few gigabytes only the shell that runs the app on a workstation needs.
{
  nixpkgs,
  rust-overlay,
  system,
  emulator ? false,
}:
let
  pkgs = import nixpkgs {
    inherit system;
    overlays = [ rust-overlay.overlays.default ];
    config = {
      android_sdk.accept_license = true;
      allowUnfree = true;
    };
  };

  # Kept in step with android/app/build.gradle.kts (compileSdk, ndkVersion).
  platform = "37";
  buildTools = "37.0.0";
  ndk = "29.0.14206865";

  android = pkgs.androidenv.composeAndroidPackages {
    # The emulator runs Android 16 (36): nixpkgs has no 37 image yet.
    minPlatformVersion = if emulator then "36" else platform;
    maxPlatformVersion = platform;
    buildToolsVersions = [ buildTools ];
    includeNDK = true;
    ndkVersions = [ ndk ];
    includeCmake = false;
    includeEmulator = emulator;
    includeSystemImages = emulator;
    # The plain AOSP image: the app depends on no Google services.
    systemImageTypes = [ "default" ];
    abiVersions = [ "x86_64" ];
  };
  sdk = "${android.androidsdk}/libexec/android-sdk";

  rust = pkgs.rust-bin.stable.latest.default.override {
    extensions = [
      "clippy"
      "rust-src"
    ];
    targets = [
      "aarch64-linux-android"
      "x86_64-linux-android"
    ];
  };
in
pkgs.mkShell {
  packages = [
    android.androidsdk
    pkgs.jdk21
    pkgs.gradle_9
    pkgs.cargo-ndk
    rust
  ];
  ANDROID_HOME = sdk;
  ANDROID_SDK_ROOT = sdk;
  ANDROID_NDK_ROOT = "${sdk}/ndk/${ndk}";
  JAVA_HOME = pkgs.jdk21.home;
  # The Android Gradle plugin fetches its own aapt2 from Maven, a binary
  # linked for an FHS system that cannot run on NixOS; the SDK's, which
  # nixpkgs patched, does the same work.
  GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdk}/build-tools/${buildTools}/aapt2";
}
