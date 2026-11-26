package org.weirdbrains.bragi

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        RustContext.init(this)
        // The relay wake path's foreground service (lib/src/wake_service.dart).
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "org.weirdbrains.bragi/wake").setMethodCallHandler { call, result ->
            when (call.method) {
                "start" -> { WakeService.start(applicationContext); result.success(null) }
                "stop" -> { WakeService.stop(applicationContext); result.success(null) }
                else -> result.notImplemented()
            }
        }
    }
}
