// Native Assets build hook: compiles crates/bragi-bridge with cargo and bundles the library
// with the app (no cargokit, no CocoaPods).
//
// The phone builds (Android, iOS) get the optional on-device model's engine (the `on-device` Cargo
// feature, llama.cpp; B4). Other targets don't, so a desktop `flutter run` skips llama.cpp's C++
// build. The `on_device` user-define in pubspec.yaml (hooks: user_defines: bragi: on_device: true or
// false) overrides that for any target.
//
// The hook runs cargo with a trimmed environment (no RUSTFLAGS, CFLAGS or ANDROID_NDK), so this
// passes what the build needs itself:
// - Build paths are remapped out of the library (Rust: --remap-path-prefix in the target's
//   rustflags; C and C++ such as llama.cpp: -ffile-prefix-map), as the dist scripts do.
// - On Android llama.cpp's CMake build uses the same NDK and API level as Flutter's own build
//   (otherwise llama-cpp-sys-2 picks the newest NDK under the SDK and API 28), with the static
//   libc++ (crates/bragi-core/Cargo.toml), so the apk needs no libc++_shared.so.
import 'dart:io';

import 'package:code_assets/code_assets.dart';
import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    final code = input.config.code;
    final triple = _rustTriple(code);
    final phone = code.targetOS == OS.android || code.targetOS == OS.iOS;
    final define = input.userDefines['on_device'];
    final onDevice = define is bool ? define : phone;

    final env = <String, String>{};
    final cargoArgs = <String>[];
    if (triple != null && !Platform.isWindows) {
      final root = Directory.fromUri(input.packageRoot.resolve('..'))
          .absolute
          .path;
      final home = Platform.environment['HOME'] ?? '';
      final cargoHome = Platform.environment['CARGO_HOME'] ?? '$home/.cargo';
      final targetDir =
          Platform.environment['CARGO_TARGET_DIR'] ??
          Directory.fromUri(input.outputDirectory.resolve('target'))
              .absolute
              .path;
      final userHome = code.targetOS == OS.iOS ? '/Users/user' : '/home/user';
      // Most specific last: rustc and gcc use the last prefix that matches (clang the longest).
      final maps = <(String, String)>[
        if (home.isNotEmpty) (_trim(home), userHome),
        (_trim(cargoHome), '/cargo'),
        (_trim(root), '/build/bragi'),
        (_trim(targetDir), '/target'),
      ];
      final rustflags = maps
          .map((m) => '"--remap-path-prefix=${m.$1}=${m.$2}"')
          .join(', ');
      cargoArgs.addAll(['--config', 'target.$triple.rustflags = [$rustflags]']);
      final cflags = maps
          .map((m) => '-ffile-prefix-map=${m.$1}=${m.$2}')
          .join(' ');
      final key = triple.replaceAll('-', '_');
      env['CFLAGS_$key'] = cflags;
      env['CXXFLAGS_$key'] = cflags;
    }
    final cc = code.targetOS == OS.android ? code.cCompiler?.compiler : null;
    if (onDevice && cc != null) {
      // <ndk>/toolchains/llvm/prebuilt/<host>/bin/clang
      env['ANDROID_NDK'] = Directory.fromUri(cc.resolve('../../../../../'))
          .absolute
          .path;
      env['ANDROID_API_LEVEL'] = '${code.android.targetNdkApi}';
    }

    await FlutterRustBridgeNativeAssetsBuilder(
      cratePath: '../crates/bragi-bridge',
      features: [if (onDevice) 'on-device'],
      extraCargoBuildArgs: cargoArgs,
      extraCargoEnvironmentVariables: env,
    ).run(input: input, output: output);
  });
}

String _trim(String path) => path.length > 1 && path.endsWith('/')
    ? path.substring(0, path.length - 1)
    : path;

/// The Rust target the builder compiles for (native_toolchain_rust's mapping).
String? _rustTriple(CodeConfig code) =>
    switch ((code.targetOS, code.targetArchitecture)) {
      (OS.android, Architecture.arm64) => 'aarch64-linux-android',
      (OS.android, Architecture.arm) => 'armv7-linux-androideabi',
      (OS.android, Architecture.x64) => 'x86_64-linux-android',
      (OS.iOS, Architecture.arm64) =>
        code.iOS.targetSdk == IOSSdk.iPhoneSimulator
            ? 'aarch64-apple-ios-sim'
            : 'aarch64-apple-ios',
      (OS.iOS, Architecture.x64) => 'x86_64-apple-ios',
      (OS.linux, Architecture.arm64) => 'aarch64-unknown-linux-gnu',
      (OS.linux, Architecture.x64) => 'x86_64-unknown-linux-gnu',
      (OS.macOS, Architecture.arm64) => 'aarch64-apple-darwin',
      (OS.macOS, Architecture.x64) => 'x86_64-apple-darwin',
      _ => null,
    };
