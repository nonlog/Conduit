# protobuf-javalite reaches generated classes reflectively.
-keep class com.conduit.sync.proto.** { *; }

# LSPosed loads this entry point by its literal name from assets/xposed_init.
# It is not referenced by the Android manifest or normal app call graph.
-keep class com.conduit.sync.ClipboardHook** { *; }
# The framework supplies this compileOnly API inside system_server.
-dontwarn de.robv.android.xposed.**

# Only the three primitives are used; let R8 drop the rest of BouncyCastle.
-dontwarn org.bouncycastle.**
