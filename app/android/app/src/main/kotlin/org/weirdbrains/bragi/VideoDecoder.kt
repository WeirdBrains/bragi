package org.weirdbrains.bragi

import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.os.Build
import android.os.Handler
import android.os.Looper
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

/**
 * The phone's video decoder for bragi-core's sampling plan (LF-43, V3; lib/src/video.dart):
 * `probe` reads a clip's length, size, frame rate and whether it has sound; `frames` decodes the
 * frames at the times the core planned, scaled to fit its size, as JPEG. MediaMetadataRetriever
 * only: no ffmpeg on phones. Work runs off the main thread; replies go back on it.
 */
object VideoDecoder : MethodChannel.MethodCallHandler {
    private val work = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        val path = call.argument<String>("path")
        if (path == null) {
            result.error("bad_request", "no clip given", null)
            return
        }
        when (call.method) {
            "probe" -> run(result) { probe(path) }
            "frames" -> {
                val times = call.argument<List<Number>>("times_ms")?.map { it.toLong() } ?: emptyList()
                val width = call.argument<Number>("width")?.toInt() ?: 0
                val height = call.argument<Number>("height")?.toInt() ?: 0
                val quality = (call.argument<Number>("quality")?.toInt() ?: 85).coerceIn(1, 100)
                run(result) { frames(path, times, width, height, quality) }
            }
            else -> result.notImplemented()
        }
    }

    private fun run(result: MethodChannel.Result, job: () -> Any) {
        work.execute {
            try {
                val out = job()
                main.post { result.success(out) }
            } catch (e: Exception) {
                main.post { result.error("video", e.message ?: "the clip can't be read as a video", null) }
            }
        }
    }

    private fun <T> open(path: String, use: (MediaMetadataRetriever) -> T): T {
        val r = MediaMetadataRetriever()
        try {
            r.setDataSource(path)
            return use(r)
        } finally {
            r.release()
        }
    }

    private fun probe(path: String): Map<String, Any?> = open(path) { r ->
        val meta = { key: Int -> r.extractMetadata(key) }
        if (meta(MediaMetadataRetriever.METADATA_KEY_HAS_VIDEO) != "yes") {
            throw IllegalArgumentException("the file has no video track")
        }
        val duration = meta(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: 0L
        var width = meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull() ?: 0
        var height = meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull() ?: 0
        val rotation = meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
        if (rotation == 90 || rotation == 270) {
            val w = width; width = height; height = w
        }
        val count = if (Build.VERSION.SDK_INT >= 28) meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)?.toLongOrNull() else null
        val fps = if (count != null && duration > 0) count * 1000.0 / duration else null
        mapOf(
            "duration_ms" to duration,
            "width" to width,
            "height" to height,
            "fps" to fps,
            "has_audio" to (meta(MediaMetadataRetriever.METADATA_KEY_HAS_AUDIO) == "yes"),
        )
    }

    private fun frames(path: String, times: List<Long>, width: Int, height: Int, quality: Int): List<Map<String, Any>> = open(path) { r ->
        times.map { t ->
            val us = t * 1000
            val frame: Bitmap = (
                if (Build.VERSION.SDK_INT >= 27 && width > 0 && height > 0) {
                    r.getScaledFrameAtTime(us, MediaMetadataRetriever.OPTION_CLOSEST, width, height)
                } else {
                    r.getFrameAtTime(us, MediaMetadataRetriever.OPTION_CLOSEST)?.let { b -> scale(b, width, height) }
                }
            ) ?: throw IllegalStateException("no frame at ${t} ms")
            val out = ByteArrayOutputStream()
            frame.compress(Bitmap.CompressFormat.JPEG, quality, out)
            frame.recycle()
            mapOf("at_ms" to t, "bytes" to out.toByteArray())
        }
    }

    private fun scale(b: Bitmap, width: Int, height: Int): Bitmap {
        if (width <= 0 || height <= 0 || (b.width <= width && b.height <= height)) return b
        val k = minOf(width.toDouble() / b.width, height.toDouble() / b.height)
        val s = Bitmap.createScaledBitmap(b, (b.width * k).toInt().coerceAtLeast(1), (b.height * k).toInt().coerceAtLeast(1), true)
        if (s != b) b.recycle()
        return s
    }
}
