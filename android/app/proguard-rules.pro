# JNA and UniFFI's bindings find their classes and fields by name.
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.** { *; }
-keep class su.libri.core.** { *; }
-dontwarn java.awt.**
# The terminal page calls these by name.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}
