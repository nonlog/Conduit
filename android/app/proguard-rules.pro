# protobuf-javalite reaches generated classes reflectively.
-keep class com.conduit.sync.proto.** { *; }

# Only the three primitives are used; let R8 drop the rest of BouncyCastle.
-dontwarn org.bouncycastle.**
