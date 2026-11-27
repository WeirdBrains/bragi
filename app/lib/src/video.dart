// Video on phones (LF-43, V3). Sampling: the platform decodes (Android MediaMetadataRetriever,
// iOS AVAssetImageGenerator, over the `org.weirdbrains.bragi/video` channel; no ffmpeg on phones),
// bragi-core plans which frames and how big (bridge `video_frame_plan`), and the decoded frames
// come back to the core (`attach_video_frames`). Playback: the platform player (video_player), one
// clip at a time with the audio files (the controller follows bragi_core's Deck rule). A Flutter
// build on a computer reads clips with ffmpeg through the core and opens them in the system
// player; there is no Flutter desktop player (the GPUI app has one).
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:video_player/video_player.dart';

/// Extensions the attach picker offers as video.
const videoExtensions = ['mp4', 'm4v', 'mov', 'webm', 'mkv', 'avi', '3gp'];

bool isVideoName(String name) {
  final dot = name.lastIndexOf('.');
  return dot >= 0 &&
      videoExtensions.contains(name.substring(dot + 1).toLowerCase());
}

/// What the platform reads from a clip.
class VideoFacts {
  const VideoFacts({
    required this.durationMs,
    required this.width,
    required this.height,
    this.fps,
    required this.hasAudio,
  });

  final int durationMs;
  final int width;
  final int height;
  final double? fps;
  final bool hasAudio;

  static VideoFacts fromMap(Map<Object?, Object?> m) => VideoFacts(
    durationMs: (m['duration_ms'] as num?)?.toInt() ?? 0,
    width: (m['width'] as num?)?.toInt() ?? 0,
    height: (m['height'] as num?)?.toInt() ?? 0,
    fps: (m['fps'] as num?)?.toDouble(),
    hasAudio: m['has_audio'] == true,
  );
}

/// A frame the platform decoded.
class DecodedFrame {
  const DecodedFrame(this.atMs, this.mime, this.bytes);

  final int atMs;
  final String mime;
  final Uint8List bytes;
}

/// Reads a clip's facts and the frames the core asked for.
abstract class VideoDecoder {
  /// The decoder for this build: the platform's on Android and iOS, a scripted one in the demo
  /// build, none elsewhere (ffmpeg through the core reads clips there).
  static VideoDecoder? create() {
    if (const bool.fromEnvironment('BRAGI_DEMO')) return ScriptedVideoDecoder();
    if (Platform.isAndroid || Platform.isIOS) return PlatformVideoDecoder();
    return null;
  }

  Future<VideoFacts> probe(String path);

  /// The frames at [timesMs], scaled to fit [width] x [height], as JPEG.
  Future<List<DecodedFrame>> frames(
    String path,
    List<int> timesMs,
    int width,
    int height,
  );
}

/// The platform's decoder over the method channel (android/.../VideoDecoder.kt,
/// ios/Runner/VideoDecoder.swift).
class PlatformVideoDecoder implements VideoDecoder {
  static const _channel = MethodChannel('org.weirdbrains.bragi/video');

  @override
  Future<VideoFacts> probe(String path) async {
    final m = await _channel.invokeMapMethod<Object?, Object?>('probe', {
      'path': path,
    });
    if (m == null) {
      throw const FormatException("the clip can't be read as a video");
    }
    return VideoFacts.fromMap(m);
  }

  @override
  Future<List<DecodedFrame>> frames(
    String path,
    List<int> timesMs,
    int width,
    int height,
  ) async {
    final list = await _channel.invokeListMethod<Object?>('frames', {
      'path': path,
      'times_ms': timesMs,
      'width': width,
      'height': height,
      'quality': 85,
    });
    return [
      for (final f in list ?? const <Object?>[])
        if (f is Map)
          DecodedFrame(
            (f['at_ms'] as num).toInt(),
            'image/jpeg',
            f['bytes'] as Uint8List,
          ),
    ];
  }
}

/// A 32x18 JPEG, the scripted decoder's every frame.
final Uint8List sampleJpeg = base64Decode(
  '/9j/4AAQSkZJRgABAgAAAQABAAD//gAPTGF2YzYzLjEuMTAyAP/bAEMACBAQExATFhYWFhYWGhgaGxsbGhoaGhsbGx0dHSIiIh0dHRsbHR0gICIiJSYlIyMiIyYmKCgoMDAuLjg4OkVFU//EAE0AAQEAAAAAAAAAAAAAAAAAAAAFAQEBAQAAAAAAAAAAAAAAAAAABQYQAQAAAAAAAAAAAAAAAAAAAAARAQAAAAAAAAAAAAAAAAAAAAD/wAARCAASACADASIAAhEAAxEA/9oADAMBAAIRAxEAPwCIA0iAAAAAAA//2Q==',
);

/// No platform: a 42 s 1920x1080 clip without sound, every frame the sample JPEG (the demo build
/// and the tests). Records what it was asked for.
class ScriptedVideoDecoder implements VideoDecoder {
  ScriptedVideoDecoder({
    this.facts = const VideoFacts(
      durationMs: 42000,
      width: 1920,
      height: 1080,
      fps: 30,
      hasAudio: false,
    ),
  });

  final VideoFacts facts;
  final asked = <(List<int>, int, int)>[];

  @override
  Future<VideoFacts> probe(String path) async => facts;

  @override
  Future<List<DecodedFrame>> frames(
    String path,
    List<int> timesMs,
    int width,
    int height,
  ) async {
    asked.add((timesMs, width, height));
    return [for (final t in timesMs) DecodedFrame(t, 'image/jpeg', sampleJpeg)];
  }
}

/// Plays one clip: the platform player, or a scripted clock.
abstract class VideoClipBackend {
  /// The player for this build: the platform's on Android and iOS, the scripted one in the demo
  /// build, none elsewhere (the clip opens in the system player).
  static VideoClipBackend? create() {
    if (const bool.fromEnvironment('BRAGI_DEMO')) {
      return ScriptedVideoClipBackend();
    }
    if (Platform.isAndroid || Platform.isIOS) return PlatformVideoClipBackend();
    return null;
  }

  /// Loads the clip at [path] and returns its length, if the player knows it. Throws when the
  /// player can't open it (the card then offers the system player).
  Future<Duration?> load(String path, Duration? known);
  Future<void> play();
  Future<void> pause();
  Future<void> seek(Duration to);
  Future<void> setMuted(bool muted);
  Future<void> stop();

  /// The play position, as it moves.
  Stream<Duration> get position;

  /// Fires when the clip plays to its end.
  Stream<void> get ended;

  /// Fires when playback fails after it started.
  Stream<String> get failed;

  /// The picture.
  Widget view();
}

/// The platform player (ExoPlayer on Android, AVPlayer on iOS).
class PlatformVideoClipBackend implements VideoClipBackend {
  VideoPlayerController? _c;
  final _position = StreamController<Duration>.broadcast();
  final _ended = StreamController<void>.broadcast();
  final _failed = StreamController<String>.broadcast();
  bool _endSent = false;

  void _listen() {
    final c = _c;
    if (c == null) return;
    final v = c.value;
    if (v.hasError) {
      _failed.add(v.errorDescription ?? 'the player stopped');
      return;
    }
    _position.add(v.position);
    final done =
        v.isInitialized &&
        v.duration > Duration.zero &&
        v.position >= v.duration &&
        !v.isPlaying;
    if (done && !_endSent) {
      _endSent = true;
      _ended.add(null);
    } else if (!done) {
      _endSent = false;
    }
  }

  @override
  Future<Duration?> load(String path, Duration? known) async {
    await stop();
    final c = VideoPlayerController.file(File(path));
    _c = c;
    await c.initialize();
    c.addListener(_listen);
    return c.value.duration > Duration.zero ? c.value.duration : known;
  }

  @override
  Future<void> play() async => _c?.play();

  @override
  Future<void> pause() async => _c?.pause();

  @override
  Future<void> seek(Duration to) async => _c?.seekTo(to);

  @override
  Future<void> setMuted(bool muted) async => _c?.setVolume(muted ? 0 : 1);

  @override
  Future<void> stop() async {
    final c = _c;
    _c = null;
    if (c != null) {
      c.removeListener(_listen);
      await c.dispose();
    }
  }

  @override
  Stream<Duration> get position => _position.stream;

  @override
  Stream<void> get ended => _ended.stream;

  @override
  Stream<String> get failed => _failed.stream;

  @override
  Widget view() {
    final c = _c;
    if (c == null || !c.value.isInitialized) return const SizedBox.shrink();
    return AspectRatio(aspectRatio: c.value.aspectRatio, child: VideoPlayer(c));
  }
}

/// No picture or sound: a timer moves the position (the demo build and the tests).
class ScriptedVideoClipBackend implements VideoClipBackend {
  Duration _at = Duration.zero;
  Duration _len = Duration.zero;
  Timer? _timer;
  bool muted = false;
  final _position = StreamController<Duration>.broadcast();
  final _ended = StreamController<void>.broadcast();
  final _failed = StreamController<String>.broadcast();

  /// Makes the next [load] fail, like a codec the platform lacks.
  String? failNextLoad;

  @override
  Future<Duration?> load(String path, Duration? known) async {
    final f = failNextLoad;
    failNextLoad = null;
    if (f != null) throw PlatformException(code: 'VideoError', message: f);
    _at = Duration.zero;
    _len = known ?? const Duration(seconds: 3);
    return _len;
  }

  @override
  Future<void> play() async {
    _timer?.cancel();
    if (_at >= _len) _at = Duration.zero;
    _timer = Timer.periodic(const Duration(milliseconds: 100), (_) {
      _at += const Duration(milliseconds: 100);
      if (_at >= _len) {
        _at = _len;
        _timer?.cancel();
        _position.add(_at);
        _ended.add(null);
      } else {
        _position.add(_at);
      }
    });
  }

  @override
  Future<void> pause() async => _timer?.cancel();

  @override
  Future<void> seek(Duration to) async {
    _at = to > _len ? _len : to;
    _position.add(_at);
  }

  @override
  Future<void> setMuted(bool muted) async => this.muted = muted;

  @override
  Future<void> stop() async {
    _timer?.cancel();
    _at = Duration.zero;
  }

  /// Fails mid-clip, like a decoder error.
  void fail(String why) => _failed.add(why);

  @override
  Stream<Duration> get position => _position.stream;

  @override
  Stream<void> get ended => _ended.stream;

  @override
  Stream<String> get failed => _failed.stream;

  @override
  Widget view() => const ColoredBox(
    key: Key('video-picture'),
    color: Color(0xFF000000),
    child: SizedBox(width: 320, height: 180),
  );
}

/// Why a clip doesn't play in the chat; each says what to do.
enum VideoFallback { notHere, noPlayer, cantPlay }

String videoFallbackMessage(VideoFallback f, [String? detail]) => switch (f) {
  VideoFallback.notHere => "The clip isn't on this device, so it can't play here. The frames the model saw are below.",
  VideoFallback.noPlayer => 'Clips play in your system player on this device.',
  VideoFallback.cantPlay =>
    "This clip didn't play here${detail == null ? '' : ' ($detail)'}. Open it in your system player.",
};

/// The clip playing or paused.
class VideoClip {
  VideoClip({required this.conversation, required this.file, this.duration});

  final String conversation;
  final String file;
  Duration? duration;
  Duration position = Duration.zero;
  bool playing = false;
  bool loading = true;
  bool muted = false;
}
