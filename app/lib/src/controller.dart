// What the screens show, kept in step with bragi-core. No behaviour of its own: every
// action is one core call, and streamed text is placed where the core says.
import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter/widgets.dart' show Widget;
import 'package:url_launcher/url_launcher.dart';

import 'api.dart';
import 'appearance.dart';
import 'audio.dart';
import 'code_api.dart';
import 'code_controller.dart';
import 'video.dart';

/// Puts [delta] at [offset] (characters, as the core counts them), replacing what follows.
String splice(String text, int offset, String delta) {
  final kept = String.fromCharCodes(text.runes.take(offset));
  return kept + delta;
}

/// A size for people: `512 B`, `12 KiB`, `3.4 MiB` (as bragi-core writes it).
String humanSize(int n) {
  if (n < 1024) return '$n B';
  if (n < 1024 * 1024) return '${(n + 1023) ~/ 1024} KiB';
  return '${(n / (1024 * 1024)).toStringAsFixed(1)} MiB';
}

/// An agent task's approval card, waiting for the user.
class ApprovalCard {
  const ApprovalCard({
    required this.requestId,
    required this.ask,
    required this.summary,
    required this.tier,
  });

  final String requestId;
  final int ask;
  final String summary;
  final String tier;
}

class BragiController extends ChangeNotifier {
  BragiController(
    this.api, {
    ClipBackend Function()? audio,
    CodeApi? code,
    VideoDecoder? Function()? decoder,
    VideoClipBackend? Function()? video,
    Future<bool> Function(String path)? openExternally,
  }) : _newAudio = audio ?? ClipBackend.create,
       _codeApi = code ?? const RustCodeApi(),
       _newDecoder = decoder ?? VideoDecoder.create,
       _newVideo = video ?? VideoClipBackend.create,
       _openExternally = openExternally ?? _systemOpen;

  final BragiApi api;
  final CodeApi _codeApi;

  /// Code mode: the project, its context, proposed changes and shell runs (the same conversation).
  late final CodeController code = CodeController(this, _codeApi);

  /// The look: the chosen theme and the palette the widgets paint with.
  late final AppearanceController appearance = AppearanceController(api);
  StateView? state;
  List<SummaryView> conversations = const [];
  ConversationView? current;

  /// Settings and sessions (LF-11): endpoints, Weirdnet links, what a session can be bound to,
  /// and the conversations with their folder and target.
  List<EndpointView> endpoints = const [];
  List<LinkView> links = const [];
  List<PoolView> pools = const [];

  /// Hide the history sidebar on a wide layout (remembered in the profile).
  bool sidebarCollapsed = false;
  List<TargetView> targets = const [];
  List<FolderView> folders = const [];
  List<SessionView> sessions = const [];

  /// Where the next new chat goes (`null`: the default); a session already bound uses its own.
  TargetView? newChatTarget;

  /// The current conversation's messages, the last one growing while it streams.
  List<MessageView> messages = const [];
  String? _model;
  String? requestId;
  String? notice;
  LinkCheckView? linkCheck;

  /// Files chosen for the next message, agent mode, and an agent task's approval card.
  List<AttachmentView> attachments = const [];
  bool agent = false;
  ApprovalCard? card;

  /// The last delete, while its Undo is offered.
  DeletedView? deleted;
  StreamSubscription<StateView>? _watch;
  final Map<String, Map<String, dynamic>> _markdown = {};

  /// A reply's Markdown as bragi-core parses it, kept per text (a streaming answer is re-parsed
  /// on each delta, tolerantly).
  Map<String, dynamic> markdown(String text, bool streaming) {
    final key = '${streaming ? 's' : 'd'}$text';
    final hit = _markdown[key];
    if (hit != null) return hit;
    if (_markdown.length > 200) _markdown.clear();
    return _markdown[key] =
        jsonDecode(api.markdownJson(text, streaming)) as Map<String, dynamic>;
  }

  /// When the streaming answer's first reasoning arrived (the Thinking section's live time).
  DateTime? thinkingSince;

  String get pairState => state?.pair.state ?? 'unpaired';

  /// The node search's line (an added node address: looking, connected, or what to check).
  String? get nodeSearchLine => state?.nodeSearchLine;
  String get nodeSearch => state?.nodeSearch ?? 'idle';
  bool get linked => pairState == 'linked';
  bool get streaming => requestId != null;
  bool get canContinue => !streaming && (current?.continuable ?? false);
  List<ModelView> get models => state?.models ?? const [];
  String? get model => (_model != null && models.any((m) => m.id == _model))
      ? _model
      : state?.defaultModel;

  Future<void> attach() async {
    await appearance.load();
    try {
      sidebarCollapsed = api.sidebarCollapsed();
    } catch (_) {}
    _watch = api.watchState().listen((s) {
      final wasLinked = linked;
      state = s;
      // History from another device arrives with a state wake-up.
      if (linked && (!wasLinked || (!streaming && !_refreshing))) {
        unawaited(_refresh());
      }
      notifyListeners();
    });
    await _refresh();
  }

  @override
  void dispose() {
    _watch?.cancel();
    unawaited(stopClip());
    unawaited(stopVideo());
    unawaited(_clipPosition?.cancel());
    unawaited(_clipEnded?.cancel());
    for (final s in _videoSubs) {
      unawaited(s.cancel());
    }
    super.dispose();
  }

  bool _refreshing = false;

  /// Bumped at the start of each history load; a slower, earlier load must not overwrite a newer one.
  int _loadRev = 0;

  /// The conversation the user last opened; a stale list refresh must not replace it with another.
  String? _openId;

  Future<void> _refresh() async {
    final rev = ++_loadRev;
    _refreshing = true;
    try {
      final conversations = await api.conversations(0, 100);
      await _refreshSettings();
      if (rev != _loadRev) return;
      this.conversations = conversations;
      final id = _openId ?? current?.id;
      if (id != null) {
        final latest = await api.conversation(id);
        if (rev != _loadRev) return;
        current = latest;
        messages = List.of(current!.messages);
      }
    } catch (e) {
      if (rev != _loadRev) return;
      notice = '$e';
    } finally {
      if (rev == _loadRev) _refreshing = false;
    }
    if (rev != _loadRev) return;
    notifyListeners();
  }

  /// Re-reads Settings' lists and the targets (the on-device model came or went).
  Future<void> refreshTargets() async {
    await _refreshSettings();
    notifyListeners();
  }

  Future<void> _refreshSettings() async {
    endpoints = await api.endpoints();
    links = await api.links();
    pools = await api.pools();
    targets = await api.targets();
    folders = await api.folders();
    sessions = await api.sessions(0, 100);
    final t = newChatTarget;
    // A target that was deleted or switched to fallback-only is no longer offered.
    if (t != null &&
        !targets.any(
          (x) =>
              x.kind == t.kind &&
              x.endpoint == t.endpoint &&
              x.model == t.model,
        )) {
      newChatTarget = null;
    }
  }

  /// The open conversation's session (its folder and target), if it has one listed.
  SessionView? get session {
    final id = current?.id;
    if (id == null) return null;
    for (final s in sessions) {
      if (s.id == id) return s;
    }
    return null;
  }

  /// The words for where the next message goes.
  String get targetLabel =>
      session?.targetLabel ?? newChatTarget?.label ?? 'Weirdnet pool';

  /// The open session lost its endpoint: the next send needs a target.
  bool get needsTarget => session?.unbound ?? false;

  void pickTarget(TargetView t) {
    final c = current;
    if (c == null) {
      newChatTarget = t;
      notifyListeners();
    } else {
      _try(() async {
        await api.bindSession(c.id, t);
        await _refresh();
      });
    }
  }

  Future<void> reloadSettings() => _try(() async {
    await _refreshSettings();
  });

  /// Adds (when [id] is null) or replaces an endpoint. Returns whether it went; the notice says why not.
  Future<bool> saveEndpoint(String? id, EndpointForm form) async {
    var ok = false;
    await _try(() async {
      if (id == null) {
        await api.addEndpoint(form);
      } else {
        await api.editEndpoint(id, form);
      }
      ok = true;
      await _refreshSettings();
    });
    return ok;
  }

  /// Deletes an endpoint; its sessions move to [rebindTo] or become unbound. History stays.
  Future<DeleteReportView?> deleteEndpoint(
    String id,
    TargetView? rebindTo,
  ) async {
    DeleteReportView? r;
    await _try(() async {
      r = await api.deleteEndpoint(id, rebindTo);
      await _refresh();
    });
    return r;
  }

  Future<void> moveEndpoint(String id, int index) => _try(() async {
    await api.moveEndpoint(id, index);
    await _refreshSettings();
  });

  Future<TestResultView?> testEndpoint(String id) async {
    TestResultView? r;
    await _try(() async => r = await api.testEndpoint(id));
    return r;
  }

  Future<TestResultView?> testAddress(String url, String? key) async {
    TestResultView? r;
    await _try(() async => r = await api.testEndpointAddress(url, key));
    return r;
  }

  Future<bool> editLink(String old, String address) async {
    var ok = false;
    await _try(() async {
      await api.editLink(old, address);
      ok = true;
      await _refreshSettings();
    });
    return ok;
  }

  Future<void> removeLink(String address) => _try(() async {
    await api.removeLink(address);
    await _refreshSettings();
  });

  Future<bool> createFolder(String name, String? parent) async {
    var ok = false;
    await _try(() async {
      await api.createFolder(name, parent);
      ok = true;
      await _refreshSettings();
    });
    return ok;
  }

  Future<void> renameFolder(String id, String name) => _try(() async {
    await api.renameFolder(id, name);
    await _refreshSettings();
  });

  Future<void> moveFolder(String id, String? parent) => _try(() async {
    await api.moveFolder(id, parent);
    await _refreshSettings();
  });

  /// Deletes a folder; what it holds moves up a level.
  Future<void> deleteFolder(String id) => _try(() async {
    await api.deleteFolder(id);
    await _refreshSettings();
  });

  Future<void> moveSession(String conversation, String? folder) =>
      _try(() async {
        await api.setSessionFolder(conversation, folder);
        await _refreshSettings();
      });

  Future<void> _try(Future<void> Function() f) async {
    try {
      notice = null;
      // Anything the user does clears the node search's line (an added address starts a new one).
      api.dismissNodeSearch();
      await f();
    } catch (e) {
      notice = '$e';
    }
    notifyListeners();
  }

  Future<void> showCode() => _try(() => api.pairOffer());
  Future<void> confirmCode(bool matches) =>
      _try(() => api.pairConfirm(matches));
  Future<void> cancelCode() => _try(() => api.pairCancel());

  void toggleSidebar() {
    sidebarCollapsed = !sidebarCollapsed;
    try {
      api.setSidebarCollapsed(sidebarCollapsed);
    } catch (e) {
      notice = '$e';
    }
    notifyListeners();
  }

  Future<void> addPool() => _try(() async {
    await api.addPool();
  });

  Future<void> switchPool(String id) => _try(() async {
    _openId = null;
    current = null;
    messages = const [];
    final p = await api.switchPool(id);
    await _refresh();
    notice = 'Switched to ${p.name}.';
  });

  Future<void> renamePool(String id, String name) => _try(() async {
    await api.renamePool(id, name);
    await _refreshSettings();
  });

  Future<void> removePool(String id) => _try(() async {
    _openId = null;
    current = null;
    messages = const [];
    await api.removePool(id);
    await _refresh();
    notice = 'Removed the pool.';
  });

  void pickModel(String id) {
    _model = id;
    notifyListeners();
  }

  void newChat() {
    _loadRev++;
    _openId = null;
    current = null;
    newChatTarget = null;
    messages = const [];
    notifyListeners();
  }

  Future<void> open(String id) => _try(() async {
    _openId = id;
    final current = await api.conversation(id);
    if (_openId != id) return;
    this.current = current;
    messages = List.of(current.messages);
  });

  /// Toggles a reaction on a message of the open conversation (the same one again removes it).
  Future<void> react(MessageView m, String emoji) => _try(() async {
    final c = current;
    if (c == null || m.id.isEmpty) return;
    await api.react(c.id, m.id, emoji);
    current = await api.conversation(c.id);
    messages = List.of(current!.messages);
  });

  Future<void> send(String prompt) async {
    if (streaming) return;
    // Words are needed unless an image or a sound is attached.
    if (prompt.trim().isEmpty && !attachments.any((a) => a.kind != 'text')) {
      return;
    }
    final files = attachments;
    attachments = const [];
    await _try(() async {
      try {
        final started = agent
            ? await api.sendAgent(current?.id, prompt, null, files)
            : await api.sendTo(
                current?.id,
                prompt,
                model,
                files,
                current == null ? newChatTarget : null,
              );
        await _follow(started);
      } catch (_) {
        attachments = files;
        rethrow;
      }
    });
  }

  void toggleAgent() {
    agent = !agent;
    notifyListeners();
  }

  /// Adds a file the picker returned: text, an image (PNG, JPEG, GIF, WebP) or audio (WAV, MP3). Another
  /// binary, or a big one, is refused with the core's words.
  void attachFile(String name, List<int> bytes) {
    try {
      attachments = [...attachments, api.attachBytes(name, bytes)];
      notice = null;
    } catch (e) {
      notice = '$e';
    }
    notifyListeners();
  }

  final Map<String, Future<Uint8List>> _bytes = {};

  /// A file's bytes in the current chat (an image preview, or the phone's save sheet), fetched once.
  Future<Uint8List> fileBytes(FileView f) {
    final c = current;
    if (c == null) return Future.error('no chat open');
    return _bytes.putIfAbsent(
      '${c.id}/${f.id}',
      () => api.fileBytes(c.id, f.id),
    );
  }

  // ---- audio files: one clip at a time, in the chat on screen (bragi_core::audio::Deck) ----

  final ClipBackend Function() _newAudio;
  ClipBackend? _audio;
  StreamSubscription<Duration>? _clipPosition;
  StreamSubscription<void>? _clipEnded;

  /// The clip playing or paused, if any.
  Clip? clip;

  /// Play or pause on an audio file's card. Another clip stops first.
  Future<void> playFile(FileView f) async {
    final conv = current?.id;
    if (conv == null) return;
    final c = clip;
    if (c != null && c.conversation == conv && c.file == f.id) {
      if (c.loading) return;
      final a = _audio!;
      if (c.playing) {
        c.playing = false;
        notifyListeners();
        await a.pause();
      } else {
        if (c.duration != null && c.position >= c.duration!) {
          c.position = Duration.zero;
          await a.seek(Duration.zero);
        }
        c.playing = true;
        notifyListeners();
        await a.play();
      }
      return;
    }
    await stopClip();
    await stopVideo();
    final known = f.durationMs == null
        ? null
        : Duration(milliseconds: f.durationMs!.toInt());
    final next = Clip(conversation: conv, file: f.id, duration: known);
    clip = next;
    notifyListeners();
    try {
      final a = _audio ??= _newAudio();
      _clipPosition ??= a.position.listen((p) {
        final c = clip;
        if (c == null || c.loading) return;
        c.position = p;
        notifyListeners();
      });
      _clipEnded ??= a.ended.listen((_) {
        final c = clip;
        if (c == null) return;
        c.playing = false;
        c.position = c.duration ?? c.position;
        notifyListeners();
      });
      final bytes = await api.fileBytes(conv, f.id);
      final length = await a.load(f.name, bytes, known);
      if (!identical(clip, next)) return;
      next
        ..duration = length ?? known
        ..loading = false
        ..playing = true;
      await a.play();
    } catch (e) {
      if (identical(clip, next)) clip = null;
      notice = 'Could not play ${f.name}: $e';
    }
    notifyListeners();
  }

  /// The seek bar of file [f], moved to [fraction] of its length.
  Future<void> seekFile(FileView f, double fraction) async {
    final c = clip;
    final len = c?.duration;
    if (c == null || c.file != f.id || c.loading || len == null) return;
    final to = Duration(
      milliseconds: (fraction.clamp(0.0, 1.0) * len.inMilliseconds).round(),
    );
    c.position = to;
    notifyListeners();
    await _audio?.seek(to);
  }

  /// Stops the clip, if one plays.
  Future<void> stopClip() async {
    if (clip == null) return;
    clip = null;
    await _audio?.stop();
  }

  // ---- video (LF-43, V3): attach through the platform decoder, play inline, one clip at a time ----

  final VideoDecoder? Function() _newDecoder;
  final VideoClipBackend? Function() _newVideo;
  final Future<bool> Function(String path) _openExternally;
  VideoDecoder? _decoder;
  VideoClipBackend? _videoBackend;
  final List<StreamSubscription<Object?>> _videoSubs = [];

  /// The clip playing or paused, if any.
  VideoClip? video;

  /// Why a clip can't play in the chat, by file id.
  final Map<String, (VideoFallback, String?)> videoFallbacks = {};

  /// A clip being read (the attach row shows it).
  String? videoReading;

  /// The current model's video limits, as last read (the chip and the limits sheet).
  VideoLimitsView? videoLimits;

  static Future<bool> _systemOpen(String path) async {
    // url_launcher opens file: URIs in the system's default app.
    return launchUrl(Uri.file(path), mode: LaunchMode.externalApplication);
  }

  /// The player for this build, made on first use (null: clips open in the system player).
  VideoClipBackend? get _videoPlayer => _videoBackend ??= _newVideo();

  TargetView? get _videoTarget => current == null ? newChatTarget : null;

  List<String> get _historyText => [for (final m in messages) m.text];

  /// Samples the clip at [path] for the current model and adds it to the next message. On a phone
  /// the platform decodes the frames the core plans; on a computer ffmpeg reads it through the
  /// core. Refusals (no limits for the model, too long, no decoder) show as the notice.
  Future<void> attachVideo(String path, {String? name}) async {
    final file = name ?? path.split(RegExp(r'[/\\]')).last;
    if (videoReading != null) return;
    videoReading = file;
    notice = null;
    notifyListeners();
    try {
      final decoder = _decoder ??= _newDecoder();
      final AttachmentView a;
      if (decoder != null) {
        final facts = await decoder.probe(path);
        final probe = VideoProbeView(
          durationMs: BigInt.from(facts.durationMs),
          width: facts.width,
          height: facts.height,
          fps: facts.fps,
          hasAudio: facts.hasAudio,
        );
        final plan = await api.videoFramePlan(
          file,
          probe,
          model,
          _videoTarget,
          _historyText,
        );
        final frames = await decoder.frames(
          path,
          [for (final t in plan.timesMs) t.toInt()],
          plan.width,
          plan.height,
        );
        a = await api.attachVideoFrames(
          file,
          path,
          probe,
          [
            for (final f in frames)
              VideoFrameIn(
                atMs: BigInt.from(f.atMs),
                mime: f.mime,
                bytes: f.bytes,
              ),
          ],
          model,
          _videoTarget,
          _historyText,
        );
      } else {
        final why = api.videoFileUnavailable();
        if (why != null) throw why;
        a = await api.attachVideoFile(path, model, _videoTarget, _historyText);
      }
      attachments = [...attachments, a];
    } catch (e) {
      notice = '$e';
      unawaited(refreshVideoLimits());
    } finally {
      videoReading = null;
      notifyListeners();
    }
  }

  /// Reads the current model's video limits (for the chip).
  Future<VideoLimitsView?> refreshVideoLimits() async {
    try {
      videoLimits = await api.videoLimits(model, _videoTarget);
    } catch (_) {
      videoLimits = null;
    }
    notifyListeners();
    return videoLimits;
  }

  /// Sets this device's video limits for the current model (`key=value …`; empty clears them).
  Future<VideoLimitsView?> setVideoLimits(String text) async {
    try {
      videoLimits = await api.setVideoLimits(model, text.trim(), _videoTarget);
      final ignored = videoLimits!.ignored;
      notice = ignored.isEmpty
          ? videoLimits!.chip
          : '${videoLimits!.chip}. Ignored (they would raise a reported limit): ${ignored.join(', ')}';
    } catch (e) {
      notice = '$e';
    }
    notifyListeners();
    return videoLimits;
  }

  final Map<String, VideoInfoView?> _videoInfo = {};

  /// The clip of file [file] in the current chat (null when it isn't one), read once.
  VideoInfoView? videoInfo(String file) {
    final c = current;
    if (c == null) return null;
    return _videoInfo.putIfAbsent('${c.id}/$file', () {
      try {
        return api.videoAttachment(c.id, file);
      } catch (_) {
        return null;
      }
    });
  }

  /// Play or pause on a clip's card; [from] plays from there (a frame of the strip). Audio stops
  /// first. When the clip can't play here, the card says why.
  Future<void> playVideo(
    String file,
    VideoInfoView info, {
    Duration? from,
  }) async {
    final conv = current?.id;
    if (conv == null) return;
    final v = video;
    final player = _videoBackend;
    if (v != null &&
        v.conversation == conv &&
        v.file == file &&
        player != null) {
      if (v.loading) return;
      if (from != null) {
        v.position = from;
        await player.seek(from);
        if (!v.playing) {
          v.playing = true;
          await player.play();
        }
      } else if (v.playing) {
        v.playing = false;
        await player.pause();
      } else {
        if (v.duration != null && v.position >= v.duration!) {
          v.position = Duration.zero;
          await player.seek(Duration.zero);
        }
        v.playing = true;
        await player.play();
      }
      notifyListeners();
      return;
    }
    await stopClip();
    await stopVideo();
    final path = info.clip;
    if (path == null || !info.clipHere) {
      videoFallbacks[file] = (VideoFallback.notHere, null);
      notifyListeners();
      return;
    }
    final p = _videoPlayer;
    if (p == null) {
      videoFallbacks[file] = (VideoFallback.noPlayer, null);
      notifyListeners();
      return;
    }
    videoFallbacks.remove(file);
    final known = Duration(milliseconds: info.durationMs.toInt());
    final next = VideoClip(conversation: conv, file: file, duration: known);
    video = next;
    notifyListeners();
    if (_videoSubs.isEmpty) {
      _videoSubs.add(
        p.position.listen((at) {
          final v = video;
          if (v == null || v.loading) return;
          v.position = at;
          notifyListeners();
        }),
      );
      _videoSubs.add(
        p.ended.listen((_) {
          final v = video;
          if (v == null) return;
          v.playing = false;
          v.position = v.duration ?? v.position;
          notifyListeners();
        }),
      );
      _videoSubs.add(
        p.failed.listen((why) {
          final v = video;
          if (v == null) return;
          videoFallbacks[v.file] = (VideoFallback.cantPlay, why);
          unawaited(stopVideo());
          notifyListeners();
        }),
      );
    }
    try {
      final length = await p.load(path, known);
      if (!identical(video, next)) return;
      next
        ..duration = length ?? known
        ..loading = false
        ..playing = true;
      if (from != null) {
        next.position = from;
        await p.seek(from);
      }
      await p.play();
    } catch (e) {
      if (identical(video, next)) video = null;
      videoFallbacks[file] = (
        VideoFallback.cantPlay,
        e is PlatformException ? e.message : '$e',
      );
    }
    notifyListeners();
  }

  /// The seek bar of clip [file], moved to [fraction] of its length.
  Future<void> seekVideo(String file, double fraction) async {
    final v = video;
    final len = v?.duration;
    if (v == null || v.file != file || v.loading || len == null) return;
    final to = Duration(
      milliseconds: (fraction.clamp(0.0, 1.0) * len.inMilliseconds).round(),
    );
    v.position = to;
    notifyListeners();
    await _videoBackend?.seek(to);
  }

  Future<void> toggleVideoMute() async {
    final v = video;
    if (v == null) return;
    v.muted = !v.muted;
    notifyListeners();
    await _videoBackend?.setMuted(v.muted);
  }

  /// The clip in the system player.
  Future<void> openVideoExternally(VideoInfoView info) async {
    final path = info.clip;
    if (path == null || !info.clipHere) {
      notice = "That clip isn't on this device.";
    } else {
      final v = video;
      if (v != null && v.playing) {
        v.playing = false;
        await _videoBackend?.pause();
      }
      final ok = await _openExternally(path);
      notice = ok ? null : 'No app here opens ${info.name}.';
    }
    notifyListeners();
  }

  /// The picture of the clip playing.
  Widget? videoView() => video == null ? null : _videoBackend?.view();

  /// Stops the clip, if one plays.
  Future<void> stopVideo() async {
    if (video == null) return;
    video = null;
    await _videoBackend?.stop();
  }

  String? _logged;

  @override
  void notifyListeners() {
    // A clip stops when the chat on screen changes.
    final c = clip;
    if (c != null && c.conversation != current?.id) unawaited(stopClip());
    final v = video;
    if (v != null && v.conversation != current?.id) unawaited(stopVideo());
    // Every notice shown is kept in the list behind the bell.
    final n = notice;
    if (n != null && n != _logged) {
      _logged = n;
      final ok =
          n.startsWith('Saved ') ||
          n.startsWith('Linked') ||
          n.startsWith('Deleted ');
      code.log(ok ? 'ok' : 'info', n);
    }
    if (n == null) _logged = null;
    super.notifyListeners();
  }

  /// Opens conversation [id] (a code turn just saved in it) and refreshes the history list.
  Future<void> showConversation(String id) async {
    _openId = id;
    final rev = ++_loadRev;
    try {
      final current = await api.conversation(id);
      final conversations = await api.conversations(0, 100);
      await _refreshSettings();
      if (_openId != id) return;
      if (rev != _loadRev) return;
      this.current = current;
      messages = List.of(current.messages);
      this.conversations = conversations;
    } catch (e) {
      if (rev != _loadRev) return;
      notice = '$e';
    }
    if (rev != _loadRev) return;
    notifyListeners();
  }

  /// Streams an answer that bragi-core keeps out of history until it is done (a code ask): the
  /// ask and the growing answer are shown here meanwhile. Returns the answer, or null when it
  /// was stopped before any text.
  Future<MessageView?> followLocal(
    String request,
    String prompt,
    String? model,
  ) async {
    requestId = request;
    thinkingSince = null;
    messages = [
      ...messages,
      localMessage('user', prompt),
      localMessage('assistant', '', state: 'streaming', model: model),
    ];
    notifyListeners();
    var failed = false;
    try {
      await for (final e in api.followChat(request)) {
        _apply(e);
        notifyListeners();
        if (e.kind == 'error') failed = true;
        if (e.kind == 'done' || e.kind == 'error') break;
      }
    } finally {
      requestId = null;
      thinkingSince = null;
      card = null;
    }
    final last = messages.last;
    if (failed && last.text.trim().isEmpty) {
      messages = messages.take(messages.length - 2).toList();
      notifyListeners();
      return null;
    }
    return last;
  }

  /// Saves a file of the current chat at [dest] (a save dialog's path, or a folder: then under
  /// its own name, never replacing one), and says where it went.
  Future<void> saveFileTo(FileView f, String dest) => _try(() async {
    final c = current;
    if (c == null) return;
    final at = await api.saveFile(c.id, f.id, dest);
    notice = 'Saved ${f.name} to $at';
  });

  /// Shows [text] in the notice line.
  void say(String text) {
    notice = text;
    notifyListeners();
  }

  void detach(int index) {
    attachments = [...attachments]..removeAt(index);
    notifyListeners();
  }

  /// Answers the approval card: `once`, `task` or `deny`.
  void answer(String choice) {
    final c = card;
    if (c == null) return;
    card = null;
    api.approve(c.requestId, c.ask, choice);
    notifyListeners();
  }

  Future<void> continueAnswer() async {
    final c = current;
    if (c == null || streaming) return;
    await _try(() async => _follow(await api.continueAnswer(c.id, _model)));
  }

  void stop() {
    final r = requestId;
    if (r != null) api.cancel(r);
  }

  /// Deletes [m] from the open conversation (and with [withReply] the answer after it), on every
  /// linked device. Not while an answer streams. True when it went.
  Future<bool> deleteMessage(MessageView m, {required bool withReply}) async {
    final c = current;
    if (c == null || streaming) return false;
    var ok = false;
    await _try(() async {
      deleted = await api.deleteMessage(c.id, m.id, withReply);
      ok = true;
      await _refresh();
    });
    return ok;
  }

  /// Marks the open conversation for training, or withdraws it (off unless set).
  Future<void> setTraining(bool on) async {
    final c = current;
    if (c == null) return;
    await _try(() async {
      await api.setTraining(c.id, on);
      current = await api.conversation(c.id);
      messages = List.of(current!.messages);
    });
  }

  /// Deletes a whole conversation from the history list.
  Future<bool> deleteConversation(String id) async {
    if (streaming && current?.id == id) return false;
    var ok = false;
    await _try(() async {
      deleted = await api.deleteConversations([id]);
      ok = true;
      if (current?.id == id) newChat();
      await _refresh();
    });
    return ok;
  }

  /// Puts the last delete back (the snackbar's Undo).
  Future<void> undoDelete() async {
    final d = deleted;
    if (d == null) return;
    deleted = null;
    await _try(() async {
      await api.undoDelete(d.undo);
      await _refresh();
    });
  }

  /// Deletes all history, everywhere, with no undo.
  Future<void> clearHistory() => _try(() async {
    if (streaming) stop();
    final n = await api.clearHistory();
    deleted = null;
    newChat();
    await _refresh();
    notice = n == 1 ? 'Deleted 1 conversation.' : 'Deleted $n conversations.';
  });

  /// The Link a device box: a link code starts a link; a node address (`/ip4/…/p2p/…`, from
  /// `ymirctl pool addresses`) is kept and dialed at once, as on the desktop. True when an
  /// address was added (the box can be cleared).
  Future<bool> linkDevice(String link) async {
    if (api.isNodeAddress(link)) {
      var added = false;
      await _try(() async {
        // Progress and the outcome arrive as the state's node search; only a refusal is a notice.
        api.addNode(link);
        added = true;
      });
      return added;
    }
    await _try(() async => linkCheck = await api.pairAccept(link));
    return false;
  }

  Future<void> linkConfirm(bool matches) => _try(() async {
    linkCheck = null;
    await api.pairAcceptConfirm(matches);
    notice = matches ? 'Linked.' : 'Cancelled.';
  });

  Future<void> _follow(ChatStarted s) async {
    requestId = s.requestId;
    thinkingSince = null;
    current = await api.conversation(s.conversationId);
    messages = List.of(current!.messages);
    conversations = await api.conversations(0, 100);
    notifyListeners();
    await for (final e in api.followChat(s.requestId)) {
      _apply(e);
      notifyListeners();
      if (e.kind == 'done' || e.kind == 'error') break;
    }
    requestId = null;
    thinkingSince = null;
    card = null;
    await _refresh();
  }

  void _apply(ChatEventView e) {
    if (messages.isEmpty || messages.last.role != 'assistant') return;
    final last = messages.last;
    MessageView with_({
      String? text,
      String? reasoning,
      BigInt? thinkingMs,
      String? model,
      String? node,
      bool? cacheHit,
      String? standin,
      int? resumes,
      List<String>? tools,
    }) => MessageView(
      id: last.id,
      hasReply: last.hasReply,
      reactions: last.reactions,
      role: last.role,
      text: text ?? last.text,
      reasoning: reasoning ?? last.reasoning,
      thinkingMs: thinkingMs ?? last.thinkingMs,
      state: last.state,
      model: model ?? last.model,
      node: node ?? last.node,
      cacheHit: cacheHit ?? last.cacheHit,
      standin: standin ?? last.standin,
      resumes: resumes ?? last.resumes,
      attachments: last.attachments,
      files: last.files,
      tools: tools ?? last.tools,
      agent: last.agent,
      mode: last.mode,
      atMs: last.atMs,
    );
    void replace(MessageView m) =>
        messages = [...messages.take(messages.length - 1), m];
    switch (e.kind) {
      case 'delta':
        messages = [
          ...messages.take(messages.length - 1),
          with_(text: splice(last.text, e.offset.toInt(), e.text)),
        ];
      case 'reasoning':
        thinkingSince ??= DateTime.now();
        replace(
          with_(reasoning: splice(last.reasoning, e.offset.toInt(), e.text)),
        );
      case 'thought':
        thinkingSince = null;
        replace(with_(thinkingMs: e.elapsedMs));
      case 'started':
        messages = [
          ...messages.take(messages.length - 1),
          with_(model: e.model, node: e.node),
        ];
      case 'meta':
        replace(with_(cacheHit: e.cacheHit, standin: e.standin));
      case 'resumed':
        replace(with_(node: e.node, resumes: last.resumes + 1));
      case 'tool':
        if (e.toolState == 'started' &&
            e.tool != null &&
            !last.tools.contains(e.tool)) {
          replace(with_(tools: [...last.tools, e.tool!]));
        }
      case 'approval':
        card = ApprovalCard(
          requestId: requestId!,
          ask: e.ask.toInt(),
          summary: e.text,
          tier: e.tier ?? 'other',
        );
      case 'notice':
        notice = e.text;
      case 'retry':
        notice = 'No node answered; trying again (${e.attempt})…';
      case 'error':
        if (e.errorKind != 'cancelled') notice = e.text;
    }
  }
}
