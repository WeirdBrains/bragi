// What the screens show, kept in step with bragi-core. No behaviour of its own: every
// action is one core call, and streamed text is placed where the core says.
import 'dart:async';

import 'package:flutter/foundation.dart';

import 'api.dart';

/// Puts [delta] at [offset] (characters, as the core counts them), replacing what follows.
String splice(String text, int offset, String delta) {
  final kept = String.fromCharCodes(text.runes.take(offset));
  return kept + delta;
}

class BragiController extends ChangeNotifier {
  BragiController(this.api);

  final BragiApi api;
  StateView? state;
  List<SummaryView> conversations = const [];
  ConversationView? current;

  /// The current conversation's messages, the last one growing while it streams.
  List<MessageView> messages = const [];
  String? _model;
  String? requestId;
  String? notice;
  LinkCheckView? linkCheck;
  StreamSubscription<StateView>? _watch;

  String get pairState => state?.pair.state ?? 'unpaired';
  bool get linked => pairState == 'linked';
  bool get streaming => requestId != null;
  bool get canContinue => !streaming && (current?.continuable ?? false);
  List<ModelView> get models => state?.models ?? const [];
  String? get model =>
      (_model != null && models.any((m) => m.id == _model)) ? _model : state?.defaultModel;

  Future<void> attach() async {
    _watch = api.watchState().listen((s) {
      final wasLinked = linked;
      state = s;
      if (!wasLinked && linked) unawaited(_refresh());
      notifyListeners();
    });
    await _refresh();
  }

  @override
  void dispose() {
    _watch?.cancel();
    super.dispose();
  }

  Future<void> _refresh() async {
    try {
      conversations = await api.conversations(0, 100);
      final c = current;
      if (c != null) {
        current = await api.conversation(c.id);
        messages = List.of(current!.messages);
      }
    } catch (e) {
      notice = '$e';
    }
    notifyListeners();
  }

  Future<void> _try(Future<void> Function() f) async {
    try {
      notice = null;
      await f();
    } catch (e) {
      notice = '$e';
    }
    notifyListeners();
  }

  Future<void> showCode() => _try(() => api.pairOffer());
  Future<void> confirmCode(bool matches) => _try(() => api.pairConfirm(matches));
  Future<void> cancelCode() => _try(() => api.pairCancel());

  void pickModel(String id) {
    _model = id;
    notifyListeners();
  }

  void newChat() {
    current = null;
    messages = const [];
    notifyListeners();
  }

  Future<void> open(String id) => _try(() async {
        current = await api.conversation(id);
        messages = List.of(current!.messages);
      });

  Future<void> send(String prompt) async {
    if (streaming || prompt.trim().isEmpty) return;
    await _try(() async => _follow(await api.send(current?.id, prompt, model)));
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

  Future<void> linkDevice(String link) => _try(() async => linkCheck = await api.pairAccept(link));

  Future<void> linkConfirm(bool matches) => _try(() async {
        linkCheck = null;
        await api.pairAcceptConfirm(matches);
        notice = matches ? 'Linked.' : 'Cancelled.';
      });

  Future<void> _follow(ChatStarted s) async {
    requestId = s.requestId;
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
    await _refresh();
  }

  void _apply(ChatEventView e) {
    if (messages.isEmpty || messages.last.role != 'assistant') return;
    final last = messages.last;
    MessageView with_({String? text, String? model, String? node, bool? cacheHit, String? standin, int? resumes}) => MessageView(
          role: last.role,
          text: text ?? last.text,
          state: last.state,
          model: model ?? last.model,
          node: node ?? last.node,
          cacheHit: cacheHit ?? last.cacheHit,
          standin: standin ?? last.standin,
          resumes: resumes ?? last.resumes,
          atMs: last.atMs,
        );
    void replace(MessageView m) => messages = [...messages.take(messages.length - 1), m];
    switch (e.kind) {
      case 'delta':
        messages = [...messages.take(messages.length - 1), with_(text: splice(last.text, e.offset.toInt(), e.text))];
      case 'started':
        messages = [...messages.take(messages.length - 1), with_(model: e.model, node: e.node)];
      case 'meta':
        replace(with_(cacheHit: e.cacheHit, standin: e.standin));
      case 'resumed':
        replace(with_(node: e.node, resumes: last.resumes + 1));
      case 'retry':
        notice = 'No node answered; trying again (${e.attempt})…';
      case 'error':
        if (e.errorKind != 'cancelled') notice = e.text;
    }
  }
}
