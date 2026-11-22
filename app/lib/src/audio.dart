// The phone's player for the audio files in a chat. Which clip plays, and when it stops, is
// bragi-core's rule (bragi_core::audio::Deck): one clip at a time, and it stops when the chat on
// screen changes; the controller follows it. The sound goes through the platform's player
// (just_audio) in the platform audio session (audio_session: Android audio focus, the iOS
// AVAudioSession), so calls and other apps pause it as they should.
import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:audio_session/audio_session.dart';
import 'package:just_audio/just_audio.dart';
import 'package:path_provider/path_provider.dart';

/// Makes the sound for one clip at a time.
abstract class ClipBackend {
  /// Loads a clip (its file name and bytes; `known`: the length bragi-core read from it) and
  /// returns its length, if the player knows it.
  Future<Duration?> load(String name, Uint8List bytes, Duration? known);

  /// The player for this build: the platform's, or the scripted one in the demo build.
  static ClipBackend create() => const bool.fromEnvironment('BRAGI_DEMO') ? ScriptedClipBackend() : PlatformClipBackend();
  Future<void> play();
  Future<void> pause();
  Future<void> seek(Duration to);
  Future<void> stop();

  /// The play position, as it moves.
  Stream<Duration> get position;

  /// Fires when the clip plays to its end.
  Stream<void> get ended;
}

/// The platform player, in the platform audio session.
class PlatformClipBackend implements ClipBackend {
  final AudioPlayer _player = AudioPlayer();
  bool _session = false;
  final _ended = StreamController<void>.broadcast();

  @override
  Future<Duration?> load(String name, Uint8List bytes, Duration? known) async {
    if (!_session) {
      final session = await AudioSession.instance;
      await session.configure(const AudioSessionConfiguration.music());
      _session = true;
      _player.playerStateStream.listen((s) {
        if (s.processingState == ProcessingState.completed) _ended.add(null);
      });
    }
    final dir = Directory('${(await getTemporaryDirectory()).path}/bragi-audio');
    await dir.create(recursive: true);
    final file = File('${dir.path}/${name.replaceAll(RegExp(r'[/\\]'), '_')}');
    await file.writeAsBytes(bytes, flush: true);
    return await _player.setFilePath(file.path) ?? known;
  }

  @override
  Future<void> play() async {
    await (await AudioSession.instance).setActive(true);
    // Completes only when playback stops, so it isn't awaited.
    unawaited(_player.play());
  }

  @override
  Future<void> pause() => _player.pause();

  @override
  Future<void> seek(Duration to) => _player.seek(to);

  @override
  Future<void> stop() => _player.stop();

  @override
  Stream<Duration> get position => _player.positionStream;

  @override
  Stream<void> get ended => _ended.stream;
}

/// No sound: a timer moves the position (the demo build and the tests, where no platform
/// player exists).
class ScriptedClipBackend implements ClipBackend {
  Duration _at = Duration.zero;
  Duration _len = Duration.zero;
  Timer? _timer;
  final _position = StreamController<Duration>.broadcast();
  final _ended = StreamController<void>.broadcast();

  @override
  Future<Duration?> load(String name, Uint8List bytes, Duration? known) async {
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
  Future<void> stop() async {
    _timer?.cancel();
    _at = Duration.zero;
  }

  @override
  Stream<Duration> get position => _position.stream;

  @override
  Stream<void> get ended => _ended.stream;
}

/// The clip playing or paused.
class Clip {
  Clip({required this.conversation, required this.file, this.duration});

  final String conversation;
  final String file;
  Duration? duration;
  Duration position = Duration.zero;
  bool playing = false;
  bool loading = true;
}
