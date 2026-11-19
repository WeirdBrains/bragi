// Native Assets build hook: compiles crates/bragi-bridge with cargo and bundles the library
// with the app (no cargokit, no CocoaPods).
import 'package:flutter_rust_bridge_hooks/flutter_rust_bridge_hooks.dart';

void main(List<String> args) async {
  await build(args, (input, output) async {
    await const FlutterRustBridgeNativeAssetsBuilder(
      cratePath: '../crates/bragi-bridge',
    ).run(input: input, output: output);
  });
}
