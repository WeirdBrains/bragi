// The seam between code mode's widgets and bragi-core's code and desktop-link calls:
// [RustCodeApi] calls the bridge; tests may pass their own. The types are the bridge's
// (lib/src/rust/api/code.dart).
import 'rust/api/code.dart' as rust;

export 'rust/api/code.dart'
    show
        AnsweredView,
        AppliedView,
        CodeAskView,
        ContextFileView,
        ContextView,
        DesktopView,
        DiffLineView,
        EntryView,
        FileDiffView,
        HunkView,
        NotificationView,
        OpenedView,
        ProjectView,
        ReviewView,
        RootView,
        ShellEventView;

abstract class CodeApi {
  // The desktop link.
  List<rust.DesktopView> desktops();
  Future<rust.DesktopView> pair(String code, String label);
  bool isPairCode(String text);
  Future<void> forget(String id);
  Future<rust.DesktopView> refresh(String id);

  /// `auto`, `direct` or `weirdnet`.
  String route();
  void setRoute(String route);

  // The project.
  bool localAvailable();
  Future<rust.OpenedView> openLocal(String path);
  Future<rust.OpenedView> openDesktop(String id, int root, String sub);
  Future<void> close();
  Future<List<rust.EntryView>> list(String rel);
  Future<rust.ContextView> setContext(List<String> targets);
  Future<rust.CodeAskView> ask(String? conversation, String prompt, String? model);
  Future<rust.AnsweredView> answered(String? conversation, String prompt, String reply, String reasoning, String? model);
  Future<rust.ReviewView?> setHunk(int file, int hunk, bool accept);
  Future<rust.AppliedView> apply();
  Future<void> discard();
  Future<rust.AppliedView> undo();
  Stream<rust.ShellEventView> shell(String id, String command);
  bool shellStop(String id);
  Future<String> recordShell(String? conversation, String transcript);

  // Prompt history (Up/Down in the message box) and the notification list behind the bell.
  String? historyUp(String key, String current);
  String? historyDown(String key);
  void historyPush(String key, String prompt);
  void notify(String level, String text);
  List<rust.NotificationView> notifications();
  int unread();
  void seen();
}

class RustCodeApi implements CodeApi {
  const RustCodeApi();

  @override
  List<rust.DesktopView> desktops() => rust.linkDesktops();
  @override
  Future<rust.DesktopView> pair(String code, String label) => rust.linkPair(code: code, label: label);
  @override
  bool isPairCode(String text) => rust.linkIsPairCode(text: text);
  @override
  Future<void> forget(String id) => rust.linkForget(id: id);
  @override
  Future<rust.DesktopView> refresh(String id) => rust.linkRefresh(id: id);
  @override
  String route() => rust.linkRoute();
  @override
  void setRoute(String route) => rust.linkSetRoute(route: route);
  @override
  bool localAvailable() => rust.codeLocalAvailable();
  @override
  Future<rust.OpenedView> openLocal(String path) => rust.codeOpenLocal(path: path);
  @override
  Future<rust.OpenedView> openDesktop(String id, int root, String sub) => rust.codeOpenDesktop(id: id, root: root, sub: sub);
  @override
  Future<void> close() => rust.codeClose();
  @override
  Future<List<rust.EntryView>> list(String rel) => rust.codeList(rel: rel);
  @override
  Future<rust.ContextView> setContext(List<String> targets) => rust.codeSetContext(targets: targets);
  @override
  Future<rust.CodeAskView> ask(String? conversation, String prompt, String? model) => rust.codeAsk(conversation: conversation, prompt: prompt, model: model);
  @override
  Future<rust.AnsweredView> answered(String? conversation, String prompt, String reply, String reasoning, String? model) =>
      rust.codeAnswered(conversation: conversation, prompt: prompt, reply: reply, reasoning: reasoning, model: model);
  @override
  Future<rust.ReviewView?> setHunk(int file, int hunk, bool accept) => rust.codeSetHunk(file: file, hunk: hunk, accept: accept);
  @override
  Future<rust.AppliedView> apply() => rust.codeApply();
  @override
  Future<void> discard() => rust.codeDiscard();
  @override
  Future<rust.AppliedView> undo() => rust.codeUndo();
  @override
  Stream<rust.ShellEventView> shell(String id, String command) => rust.codeShell(id: id, command: command);
  @override
  bool shellStop(String id) => rust.codeShellStop(id: id);
  @override
  Future<String> recordShell(String? conversation, String transcript) => rust.codeRecordShell(conversation: conversation, transcript: transcript);
  @override
  String? historyUp(String key, String current) => rust.promptHistoryUp(key: key, current: current);
  @override
  String? historyDown(String key) => rust.promptHistoryDown(key: key);
  @override
  void historyPush(String key, String prompt) => rust.promptHistoryPush(key: key, prompt: prompt);
  @override
  void notify(String level, String text) => rust.notify(level: level, text: text);
  @override
  List<rust.NotificationView> notifications() => rust.notifications();
  @override
  int unread() => rust.notificationsUnread();
  @override
  void seen() => rust.notificationsSeen();
}
