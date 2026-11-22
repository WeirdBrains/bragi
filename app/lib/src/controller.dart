// What the screens show, kept in step with bragi-core. No behaviour of its own: every
// action is one core call, and streamed text is placed where the core says.
import 'dart:async';
import 'dart:convert';

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

  /// Settings and sessions (LF-11): endpoints, Weirdnet links, what a session can be bound to,
  /// and the conversations with their folder and target.
  List<EndpointView> endpoints = const [];
  List<LinkView> links = const [];
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
    return _markdown[key] = jsonDecode(api.markdownJson(text, streaming)) as Map<String, dynamic>;
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
      await _refreshSettings();
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

  Future<void> _refreshSettings() async {
    endpoints = await api.endpoints();
    links = await api.links();
    targets = await api.targets();
    folders = await api.folders();
    sessions = await api.sessions(0, 100);
    final t = newChatTarget;
    // A target that was deleted or switched to fallback-only is no longer offered.
    if (t != null && !targets.any((x) => x.kind == t.kind && x.endpoint == t.endpoint && x.model == t.model)) newChatTarget = null;
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
  String get targetLabel => session?.targetLabel ?? newChatTarget?.label ?? 'Weirdnet pool';

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
  Future<DeleteReportView?> deleteEndpoint(String id, TargetView? rebindTo) async {
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

  Future<void> moveSession(String conversation, String? folder) => _try(() async {
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
  Future<void> confirmCode(bool matches) => _try(() => api.pairConfirm(matches));
  Future<void> cancelCode() => _try(() => api.pairCancel());

  void pickModel(String id) {
    _model = id;
    notifyListeners();
  }

  void newChat() {
    current = null;
    newChatTarget = null;
    messages = const [];
    notifyListeners();
  }

  Future<void> open(String id) => _try(() async {
    current = await api.conversation(id);
    messages = List.of(current!.messages);
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
    if (prompt.trim().isEmpty && !attachments.any((a) => a.kind != 'text')) return;
    final files = attachments;
    attachments = const [];
    await _try(() async {
      try {
        final started = agent
            ? await api.sendAgent(current?.id, prompt, null, files)
            : await api.sendTo(current?.id, prompt, model, files, current == null ? newChatTarget : null);
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
          tools: tools ?? last.tools,
          agent: last.agent,
          atMs: last.atMs,
        );
    void replace(MessageView m) => messages = [...messages.take(messages.length - 1), m];
    switch (e.kind) {
      case 'delta':
        messages = [...messages.take(messages.length - 1), with_(text: splice(last.text, e.offset.toInt(), e.text))];
      case 'reasoning':
        thinkingSince ??= DateTime.now();
        replace(with_(reasoning: splice(last.reasoning, e.offset.toInt(), e.text)));
      case 'thought':
        thinkingSince = null;
        replace(with_(thinkingMs: e.elapsedMs));
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
