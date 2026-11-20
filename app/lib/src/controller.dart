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

/// An agent task's approval card, waiting for the user.
class ApprovalCard {
  const ApprovalCard({required this.requestId, required this.ask, required this.summary, required this.tier});

  final String requestId;
  final int ask;
  final String summary;
  final String tier;
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

  /// Files chosen for the next message, agent mode, and an agent task's approval card.
  List<AttachmentView> attachments = const [];
  bool agent = false;
  ApprovalCard? card;
  StreamSubscription<StateView>? _watch;

  String get pairState => state?.pair.state ?? 'unpaired';

  /// The node search's line (an added node address: looking, connected, or what to check).
  String? get nodeSearchLine => state?.nodeSearchLine;
  String get nodeSearch => state?.nodeSearch ?? 'idle';
  bool get linked => pairState == 'linked';
  bool get streaming => requestId != null;
  bool get canContinue => !streaming && (current?.continuable ?? false);
  List<ModelView> get models => state?.models ?? const [];
  String? get model => (_model != null && models.any((m) => m.id == _model)) ? _model : state?.defaultModel;

  Future<void> attach() async {
    _watch = api.watchState().listen((s) {
      final wasLinked = linked;
      state = s;
      // History from another device arrives with a state wake-up.
      if (linked && (!wasLinked || (!streaming && !_refreshing))) unawaited(_refresh());
      notifyListeners();
    });
    await _refresh();
  }

  @override
  void dispose() {
    _watch?.cancel();
    super.dispose();
  }

  bool _refreshing = false;

  Future<void> _refresh() async {
    _refreshing = true;
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
    _refreshing = false;
    notifyListeners();
  }

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
    final files = attachments;
    attachments = const [];
    await _try(() async {
      try {
        final started = agent
            ? await api.sendAgent(current?.id, prompt, null, files)
            : await api.sendWith(current?.id, prompt, model, files);
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

  /// Adds a text file the picker returned. A binary or big one is refused with the core's words.
  void attachFile(String name, List<int> bytes) {
    try {
      attachments = [...attachments, api.attachBytes(name, bytes)];
      notice = null;
    } catch (e) {
      notice = '$e';
    }
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
    card = null;
    await _refresh();
  }

  void _apply(ChatEventView e) {
    if (messages.isEmpty || messages.last.role != 'assistant') return;
    final last = messages.last;
    MessageView with_({String? text, String? model, String? node, bool? cacheHit, String? standin, int? resumes, List<String>? tools}) =>
        MessageView(
          role: last.role,
          text: text ?? last.text,
          state: last.state,
          model: model ?? last.model,
          node: node ?? last.node,
          cacheHit: cacheHit ?? last.cacheHit,
          standin: standin ?? last.standin,
          resumes: resumes ?? last.resumes,
          attachments: last.attachments,
          tools: tools ?? last.tools,
          agent: last.agent,
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
      case 'tool':
        if (e.toolState == 'started' && e.tool != null && !last.tools.contains(e.tool)) {
          replace(with_(tools: [...last.tools, e.tool!]));
        }
      case 'approval':
        card = ApprovalCard(requestId: requestId!, ask: e.ask.toInt(), summary: e.text, tier: e.tier ?? 'other');
      case 'notice':
        notice = e.text;
      case 'retry':
        notice = 'No node answered; trying again (${e.attempt})…';
      case 'error':
        if (e.errorKind != 'cancelled') notice = e.text;
    }
  }
}
