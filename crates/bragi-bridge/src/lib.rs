//! The Flutter app's bridge: thin conversions between `bragi-core` and Dart. No behaviour.

#[cfg(target_os = "android")]
mod android;
pub mod api;
mod frb_generated;
