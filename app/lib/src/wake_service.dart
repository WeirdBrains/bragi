// The relay wake path's Android foreground service (B4): while "Wake through your relay" is
// on, it keeps the process, and so the core's connection to the home relay, alive in the
// background (android/.../WakeService.kt). Elsewhere it does nothing: the desktop keeps the
// connection while the app runs, and iOS needs APNs for background wake.
import 'dart:io';

import 'package:flutter/services.dart';

const _channel = MethodChannel('org.weirdbrains.bragi/wake');

/// Starts or stops the foreground service to match [relayActive] (push and relay wake on).
Future<void> syncWakeService(bool relayActive) async {
  if (!Platform.isAndroid) return;
  try {
    await _channel.invokeMethod<void>(relayActive ? 'start' : 'stop');
  } on MissingPluginException {
    // Tests and other embedders have no channel.
  } on PlatformException {
    // Android refused the service (for example started from the background); the core keeps
    // the connection while the app is open.
  }
}
