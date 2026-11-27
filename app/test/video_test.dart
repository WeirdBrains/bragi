// Video in the app (LF-43, V3):
//   - without the bridge: which files are video, the scripted decoder and player, the fallbacks;
//   - through the real bridge and bragi-core against the in-process demo node: a model without
//     limits refuses a clip and the chip says why, the limits sheet sets them, the clip is
//     sampled from the frames the core planned (the scripted decoder stands in for the platform),
//     it goes with a question, and its card plays it inline (a scripted player), seeks from a
//     frame, mutes, opens in the system player, and says why when it can't play here.
//
//   cargo build -p bragi-bridge
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test --dart-define=BRAGI_DEMO=true test/video_test.dart
import 'dart:io';

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:bragi/src/video.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

void main() {
  group('without the bridge', () {
    test('video files by extension', () {
      expect(isVideoName('trip.MP4'), isTrue);
      expect(isVideoName('a.b.webm'), isTrue);
      expect(isVideoName('notes.md'), isFalse);
      expect(isVideoName('mp4'), isFalse);
    });

    test('the scripted decoder records what it was asked for', () async {
      final d = ScriptedVideoDecoder();
      final f = await d.probe('/x.mp4');
      expect(
        (f.durationMs, f.width, f.height, f.hasAudio),
        (42000, 1920, 1080, false),
      );
      final frames = await d.frames('/x.mp4', [1000, 2000], 768, 432);
      expect(frames.map((f) => f.atMs), [1000, 2000]);
      expect(frames.first.mime, 'image/jpeg');
      expect(frames.first.bytes.take(2), [0xFF, 0xD8], reason: 'a JPEG');
      expect(d.asked.single, ([1000, 2000], 768, 432));
    });

    test('the scripted player moves, ends, and fails on request', () async {
      final p = ScriptedVideoClipBackend();
      final ended = p.ended.first;
      expect(
        await p.load('/x.mp4', const Duration(milliseconds: 300)),
        const Duration(milliseconds: 300),
      );
      await p.play();
      await ended.timeout(const Duration(seconds: 3));
      p.failNextLoad = 'no decoder';
      await expectLater(
        p.load('/x.mp4', null),
        throwsA(isA<PlatformException>()),
      );
      expect(
        await p.load('/x.mp4', null),
        const Duration(seconds: 3),
        reason: 'only the next load fails',
      );
      await p.stop();
    });

    test('each fallback says what to do', () {
      expect(
        videoFallbackMessage(VideoFallback.notHere),
        contains("isn't on this device"),
      );
      expect(
        videoFallbackMessage(VideoFallback.noPlayer),
        contains('system player'),
      );
      expect(
        videoFallbackMessage(VideoFallback.cantPlay, 'no codec'),
        allOf(contains('(no codec)'), contains('system player')),
      );
    });
  });

  group('through the bridge', () {
    late Directory dir;

    setUpAll(() async {
      final lib = Platform.environment['BRAGI_BRIDGE_LIB'];
      await RustLib.init(
        externalLibrary: lib == null ? null : ExternalLibrary.open(lib),
      );
      dir = Directory.systemTemp.createTempSync('bragi-video-test');
    });

    tearDownAll(() async {
      await rust.shutdown();
      dir.deleteSync(recursive: true);
    });

    testWidgets('limits, attach a clip, ask, play it inline, fall back', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(412, 1400);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      await tester.runAsync(
        () => rust.startDemo(
          dir: dir.path,
          label: 'Bragi video test',
          acceptAfterMs: 200,
          wordDelayMs: 10,
        ),
      );
      final decoder = ScriptedVideoDecoder();
      final player = ScriptedVideoClipBackend();
      final opened = <String>[];
      final c = BragiController(
        const RustBragiApi(),
        decoder: () => decoder,
        video: () => player,
        openExternally: (path) async {
          opened.add(path);
          return true;
        },
      );
      await tester.runAsync(c.attach);
      await tester.pumpWidget(BragiApp(controller: c));

      Future<void> until(String what, bool Function() done) async {
        for (var i = 0; i < 600; i++) {
          if (done()) return;
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 50)),
          );
          await tester.pump();
        }
        fail('timed out waiting for $what');
      }

      bool shown(String key) => find.byKey(Key(key)).evaluate().isNotEmpty;

      await until('the pairing screen', () => shown('show-code'));
      await tester.tap(find.byKey(const Key('show-code')));
      await until('the check code', () => shown('codes-match'));
      await tester.tap(find.byKey(const Key('codes-match')));
      await until('the models', () => c.linked && shown('model-gemma-4-e4b'));

      // Without limits the model can't take a clip; the chip says so and the sheet sets them.
      await tester.runAsync(() => c.setVideoLimits(''));
      await tester.pump();
      if (!(c.videoLimits?.ready ?? false)) {
        await tester.tap(find.byKey(const Key('attach-video')));
        await until(
          'the refusal',
          () =>
              c.videoReading == null &&
              c.videoLimits != null &&
              c.notice != null,
        );
        expect(c.attachments, isEmpty);
        expect(shown('video-limits'), isTrue);
      }
      await tester.tap(find.byKey(const Key('video-limits')));
      await until('the limits sheet', () => shown('video-limits-field'));
      await tester.enterText(
        find.byKey(const Key('video-limits-field')),
        'context=32768 image_tokens=256 max_images=6',
      );
      await tester.tap(find.byKey(const Key('video-limits-save')));
      await until('the limits', () => c.videoLimits?.ready ?? false);
      expect(c.videoLimits!.user, contains('image_tokens=256'));

      // The clip: the core plans, the (scripted) platform decodes those frames.
      decoder.asked.clear();
      await tester.tap(find.byKey(const Key('attach-video')));
      await until(
        'the clip chip',
        () => c.videoReading == null && c.attachments.isNotEmpty,
      );
      expect(c.attachments.single.kind, 'video');
      expect(c.attachments.single.name, 'sample-clip.mp4');
      final (times, width, height) = decoder.asked.single;
      expect(times, hasLength(6), reason: 'max_images=6');
      expect(times.first, greaterThan(0));
      expect(width * 1080, height * 1920, reason: 'the shape kept');
      expect(find.textContaining('sample-clip.mp4 (video)'), findsOneWidget);

      await tester.enterText(
        find.byKey(const Key('prompt')),
        'What happens in this clip?',
      );
      await tester.tap(find.byKey(const Key('send')));
      await until(
        'the answer',
        () =>
            !c.streaming &&
            c.messages.length == 2 &&
            c.messages.last.state == 'complete',
      );
      expect(c.attachments, isEmpty);
      final file = c.messages.first.files.single.id;
      await until('the card', () => shown('video-$file'));
      final info = c.videoInfo(file)!;
      expect(info.clipHere, isTrue);
      expect(info.frames, hasLength(6));
      expect(find.byKey(Key('vbubble-$file')), findsOneWidget);
      expect(find.textContaining('sample-clip.mp4'), findsWidgets);
      expect(shown('vframe-$file-5'), isTrue);

      // Play, pause.
      await tester.tap(find.byKey(Key('vplay-$file')));
      await until(
        'playing',
        () => c.video?.playing == true && c.video!.position > Duration.zero,
      );
      expect(shown('video-picture'), isTrue);
      await tester.tap(find.byKey(Key('vplay-$file')));
      await tester.pump();
      expect(c.video!.playing, isFalse);

      // A frame plays from its time; mute.
      final at = Duration(milliseconds: info.frames[3].atMs.toInt());
      await tester.tap(find.byKey(Key('vframe-$file-3')));
      await until(
        'the seek',
        () => c.video!.playing && c.video!.position >= at,
      );
      await tester.tap(find.byKey(Key('vmute-$file')));
      await tester.pump();
      expect(player.muted, isTrue);

      // The system player pauses the inline one.
      await tester.tap(find.byKey(Key('vopen-$file')));
      await until('the system player', () => opened.isNotEmpty);
      expect(opened.single, info.clip);
      expect(c.video!.playing, isFalse);

      // A clip the platform can't play says so and keeps the system player.
      await tester.runAsync(c.stopVideo);
      player.failNextLoad = 'no decoder for this codec';
      await tester.tap(find.byKey(Key('vplay-$file')));
      await until('the fallback', () => shown('vfallback-$file'));
      expect(find.textContaining('no decoder for this codec'), findsOneWidget);
      expect(shown('vopen-$file'), isTrue);
      expect(c.video, isNull);

      // A failure mid-clip does too.
      c.videoFallbacks.clear();
      await tester.pump();
      await tester.tap(find.byKey(Key('vplay-$file')));
      await until('playing again', () => c.video?.playing == true);
      player.fail('decoder stopped');
      await until('the fallback', () => shown('vfallback-$file'));
      expect(find.textContaining('decoder stopped'), findsOneWidget);

      // A clip from another device: the frames, and why it can't play.
      final elsewhere = VideoInfoView(
        name: info.name,
        bubble: info.bubble,
        summary: info.summary,
        durationMs: info.durationMs,
        sound: info.sound,
        clip: '/elsewhere/clip.mp4',
        clipHere: false,
        frames: info.frames,
      );
      await tester.runAsync(() => c.playVideo(file, elsewhere));
      await tester.pump();
      expect(c.videoFallbacks[file]!.$1, VideoFallback.notHere);
      await tester.runAsync(() => c.openVideoExternally(elsewhere));
      expect(c.notice, contains("isn't on this device"));
      expect(opened, hasLength(1));

      // Leaving the chat stops the clip.
      c.videoFallbacks.clear();
      await tester.tap(find.byKey(Key('vplay-$file')));
      await until('playing once more', () => c.video?.playing == true);
      c.newChat();
      await tester.pump();
      expect(c.video, isNull);
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 200)),
      );
    });
  });
}
