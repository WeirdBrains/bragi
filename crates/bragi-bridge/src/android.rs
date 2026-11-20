//! Android only: gives ndk-context the process's JavaVM and application Context, through
//! `weirdnet-net` 0.8.1's `android_context_init!` (it replaces the copy Bragi carried from Morph).
//! hickory-resolver (libp2p's DNS transport inside weirdnet-net) asks the ConnectivityManager for DNS
//! servers through ndk-context and panics if it was never set, which takes down every real-network node
//! (QR device linking). `RustContext.kt` calls this once, before Dart touches the bridge.

weirdnet_net::android_context_init!(Java_org_weirdbrains_bragi_RustContext_nativeInit);
