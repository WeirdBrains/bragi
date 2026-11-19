# third_party

Patched copies of published crates. Each entry says what changed and when it goes away.

## libp2p-swarm 0.48.0

The same copy Morph carries (`WeirdBrains/morph/third_party/libp2p-swarm`). The published crate
is unchanged except for one line in `Cargo.toml`. Its wasm-only dependency
`wasm-bindgen-futures = "=0.4.58"`, which pins `js-sys` 0.3.85, is relaxed to `0.4.58`, because
GPUI's wgpu needs `js-sys` 0.3.104 or later. Cargo resolves one lockfile for every target, so
the wasm-only pin blocks the desktop shell even though neither crate builds for wasm. The
`tests/` and `benches/` directories are left out, and `#![allow(warnings)]` tops `src/lib.rs`.

This is generic to every Weirdnet app with a GPUI shell. Drop it when a libp2p-swarm release
stops pinning `wasm-bindgen-futures` exactly, or when `weirdnet-net` moves past libp2p 0.57.
