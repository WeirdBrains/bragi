// Code mode's state, beside the chat's (BragiController): the project (a folder here or on a
// paired desktop), the context files, the proposed changes and their review, and shell runs.
// No behaviour of its own: every action is one bragi-core call through [CodeApi].
import 'package:flutter/foundation.dart';

import 'api.dart';
import 'code_api.dart';
import 'controller.dart';

/// A shell run in the project, as it streams.
class ShellRun {
  ShellRun(this.id, this.command);

  final String id;
  final String command;
  final StringBuffer output = StringBuffer();

  /// The desktop is asking its user before running it.
  bool waiting = false;
  bool done = false;
  int? code;
  bool stopped = false;
  bool timedOut = false;
  String? denied;

  String get status {
    if (denied != null) return 'refused: $denied';
    if (!done) return waiting ? 'waiting for a yes on the desktop…' : 'running…';
    if (stopped) return 'stopped';
    if (timedOut) return 'timed out';
    return code == null ? 'ended' : 'exit $code';
  }

  /// What is saved in the conversation: the command, its output and how it ended.
  String get transcript => '\$ $command\n${output.toString().trimRight()}\n[$status]';
}

class CodeController extends ChangeNotifier {
  CodeController(this.chat, this.api);

  final BragiController chat;
  final CodeApi api;

  /// The message box sends code asks (true) or chat messages.
  bool on = false;
  ProjectView? project;
  ContextView? context;
  ReviewView? review;

  /// Files the last apply wrote (its Undo is offered).
  List<String> applied = const [];
  ShellRun? shell;
  bool busy = false;

  List<DesktopView> get desktops {
    try {
      return api.desktops();
    } catch (_) {
      return const [];
    }
  }

  bool get localAvailable {
    try {
      return api.localAvailable();
    } catch (_) {
      return false;
    }
  }

  /// The prompt history this box browses: the chat's, or the open project's.
  String get historyKey => on && project != null ? 'code:${project!.place}' : 'chat';

  void setMode(bool code) {
    if (on == code) return;
    on = code;
    notifyListeners();
    chat.notifyListeners();
  }

  Future<bool> _try(Future<void> Function() f) async {
    try {
      busy = true;
      notifyListeners();
      await f();
      return true;
    } catch (e) {
      chat.say('$e');
      return false;
    } finally {
      busy = false;
      notifyListeners();
    }
  }

  void _opened(OpenedView o) {
    project = o.project;
    context = null;
    review = null;
    applied = const [];
    if (o.note.isNotEmpty) chat.say(o.note);
    notify('ok', 'Opened ${o.project.name} (${o.project.place})');
  }

  Future<bool> openLocal(String path) => _try(() async => _opened(await api.openLocal(path)));

  Future<bool> openDesktop(String id, int root, String sub) => _try(() async => _opened(await api.openDesktop(id, root, sub)));

  Future<void> closeProject() => _try(() async {
    await api.close();
    project = null;
    context = null;
    review = null;
    applied = const [];
  });

  Future<List<EntryView>> list(String rel) => api.list(rel);

  Future<bool> setContext(List<String> targets) => _try(() async {
    context = await api.setContext(targets);
    if (context!.skipped.isNotEmpty) chat.say('Left out: ${context!.skipped.join('; ')}');
  });

  // ---- the desktop link ----

  Future<DesktopView?> pair(String code, String label) async {
    DesktopView? d;
    await _try(() async {
      d = await api.pair(code.trim(), label);
      notify('ok', 'Paired with ${d!.label}');
    });
    return d;
  }

  /// Whether [text] is a desktop's pairing code (a scanned QR code, or the typed one).
  bool isPairCode(String text) {
    try {
      return api.isPairCode(text);
    } catch (_) {
      return false;
    }
  }

  Future<void> forget(String id) => _try(() async {
    await api.forget(id);
    if (project?.desktopId == id) {
      await api.close();
      project = null;
    }
  });

  Future<void> refresh(String id) => _try(() async => api.refresh(id));

  String get route {
    try {
      return api.route();
    } catch (_) {
      return 'auto';
    }
  }

  void setRoute(String r) {
    try {
      api.setRoute(r);
    } catch (e) {
      chat.say('$e');
    }
    notifyListeners();
  }

  // ---- asking for changes ----

  /// Sends [prompt] as a code ask in the open conversation: the answer streams into the chat
  /// like any other, then its proposed changes come up for review.
  Future<void> ask(String prompt) async {
    if (chat.streaming || prompt.trim().isEmpty) return;
    if (project == null) {
      chat.say('Open a project first (the folder button).');
      return;
    }
    final conv = chat.current?.id;
    final model = chat.model;
    try {
      chat.notice = null;
      final a = await api.ask(conv, prompt, model);
      if (a.note.isNotEmpty) chat.notice = a.note;
      review = null;
      final reply = await chat.followLocal(a.started.requestId, prompt, model);
      if (reply == null) return;
      final r = await api.answered(conv, prompt, reply.text, reply.reasoning, reply.model ?? model);
      review = r.review;
      if (r.review.note != null) chat.notice = r.review.note;
      if (r.review.files.isNotEmpty) notify('ask', '${r.review.files.length} file(s) to review in ${project!.name}');
      await chat.showConversation(r.conversationId);
    } catch (e) {
      chat.say('$e');
    }
    notifyListeners();
  }

  Future<void> setHunk(int file, int hunk, bool accept) async {
    final r = await api.setHunk(file, hunk, accept);
    if (r != null) review = r;
    notifyListeners();
  }

  Future<void> apply() => _try(() async {
    final a = await api.apply();
    applied = a.files;
    review = null;
    notify('ok', 'Wrote ${a.files.join(', ')}');
  });

  Future<void> discard() => _try(() async {
    await api.discard();
    review = null;
  });

  Future<void> undo() => _try(() async {
    final a = await api.undo();
    applied = const [];
    notify('ok', 'Put back ${a.files.join(', ')}');
  });

  // ---- shell runs ----

  Future<void> runShell(String command) async {
    if (command.trim().isEmpty || (shell != null && !shell!.done)) return;
    if (project == null) {
      chat.say('Open a project first (the folder button).');
      return;
    }
    final run = ShellRun('sh-${DateTime.now().microsecondsSinceEpoch}', command.trim());
    shell = run;
    notifyListeners();
    try {
      await for (final e in api.shell(run.id, run.command)) {
        switch (e.kind) {
          case 'waiting':
            run.waiting = true;
          case 'out' || 'err':
            run.waiting = false;
            run.output.write(e.text);
          case 'denied':
            run.denied = e.text;
            run.done = true;
          case 'exit':
            if (e.text.isNotEmpty) run.output.write(e.text);
            run
              ..code = e.code
              ..stopped = e.stopped
              ..timedOut = e.timedOut
              ..done = true;
        }
        notifyListeners();
        if (run.done) break;
      }
    } catch (e) {
      run.output.write('$e\n');
    }
    run.done = true;
    notifyListeners();
    try {
      final id = await api.recordShell(chat.current?.id, run.transcript);
      await chat.showConversation(id);
    } catch (e) {
      chat.say('$e');
    }
  }

  void stopShell() {
    final s = shell;
    if (s != null && !s.done) api.shellStop(s.id);
  }

  void closeShell() {
    if (shell?.done ?? true) shell = null;
    notifyListeners();
  }

  // ---- prompt history and notifications ----

  String? historyUp(String current) {
    try {
      return api.historyUp(historyKey, current);
    } catch (_) {
      return null;
    }
  }

  String? historyDown() {
    try {
      return api.historyDown(historyKey);
    } catch (_) {
      return null;
    }
  }

  void historyPush(String prompt) {
    if (prompt.trim().isEmpty) return;
    try {
      api.historyPush(historyKey, prompt);
    } catch (_) {}
  }

  void notify(String level, String text) {
    log(level, text);
    notifyListeners();
  }

  /// Keeps a notice in the list without a rebuild (the chat's own notices: the chat rebuilds).
  void log(String level, String text) {
    try {
      api.notify(level, text);
    } catch (_) {}
  }

  List<NotificationView> get notifications {
    try {
      return api.notifications();
    } catch (_) {
      return const [];
    }
  }

  int get unread {
    try {
      return api.unread();
    } catch (_) {
      return 0;
    }
  }

  void seen() {
    try {
      api.seen();
    } catch (_) {}
    notifyListeners();
  }
}

/// A message the app shows before bragi-core saves it (a code ask while it streams).
MessageView localMessage(String role, String text, {String state = 'complete', String? model}) => MessageView(
  id: '',
  hasReply: false,
  reactions: const [],
  role: role,
  text: text,
  reasoning: '',
  thinkingMs: null,
  state: state,
  model: model,
  node: null,
  cacheHit: false,
  standin: null,
  resumes: 0,
  attachments: const [],
  files: const [],
  tools: const [],
  agent: false,
  mode: 'code',
  atMs: DateTime.now().millisecondsSinceEpoch,
);
