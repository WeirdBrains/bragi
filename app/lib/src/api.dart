// The seam between the widgets and bragi-core: [RustBragiApi] calls the bridge; tests may
// pass their own. The data types are the bridge's (lib/src/rust/api/bragi.dart).
import 'dart:typed_data';

import 'rust/api/bragi.dart' as rust;

export 'rust/api/bragi.dart'
    show
        AttachmentView,
        ChatEventView,
        ChatStarted,
        ConversationView,
        DeleteReportView,
        DeletedView,
        EndpointForm,
        EndpointView,
        FileView,
        FolderView,
        HubHitView,
        LinkView,
        PaletteView,
        RoleColorView,
        RoleEdit,
        RoleInfoView,
        SessionView,
        TargetView,
        ThemePrefView,
        ThemeRowView,
        TestResultView,
        LinkCheckView,
        MessageView,
        ModelNodeView,
        ModelView,
        ModelsEventView,
        NodeFitView,
        NodeModelsView,
        QuantFitView,
        StoredModelView,
        PairView,
        StateView,
        SummaryView;

abstract class BragiApi {
  Stream<rust.StateView> watchState();
  Future<String> pairOffer();
  Future<void> pairConfirm(bool matches);
  Future<void> pairCancel();
  Future<rust.LinkCheckView> pairAccept(String link);
  bool isNodeAddress(String text);
  String thinkingLabel(int elapsedMs, bool done);
  String addNode(String address);
  List<String> peers();
  void dismissNodeSearch();
  Future<void> pairAcceptConfirm(bool matches);
  Future<rust.ChatStarted> send(String? conversation, String prompt, String? model);
  Future<rust.ChatStarted> sendWith(String? conversation, String prompt, String? model, List<rust.AttachmentView> files);
  Future<rust.ChatStarted> sendAgent(String? conversation, String prompt, String? node, List<rust.AttachmentView> files);
  rust.AttachmentView attachBytes(String name, List<int> bytes);
  bool approve(String requestId, int ask, String choice);
  Future<rust.ChatStarted> continueAnswer(String conversation, String? model);
  Stream<rust.ChatEventView> followChat(String requestId);
  bool cancel(String requestId);
  Future<List<rust.SummaryView>> conversations(int offset, int limit);
  Future<rust.ConversationView> conversation(String id);
  Future<Uint8List> fileBytes(String conversation, String file);
  Future<String> saveFile(String conversation, String file, String dest);
  Future<rust.DeletedView> deleteMessage(String conversation, String id, bool withReply);
  Future<rust.DeletedView> deleteConversations(List<String> ids);
  Future<void> undoDelete(String undo);
  Future<int> clearHistory();
  Future<List<String>> react(String conversation, String message, String emoji);
  List<String> quickReactions();
  Future<void> paused();
  Future<void> resumed();
  /// A reply's Markdown parsed by bragi-core, as JSON (see `markdown.dart`).
  String markdownJson(String text, bool streaming);

  // Settings: endpoints, Weirdnet links, targets, sessions and folders.
  Future<List<rust.EndpointView>> endpoints();
  Future<rust.EndpointView> addEndpoint(rust.EndpointForm form);
  Future<rust.EndpointView> editEndpoint(String id, rust.EndpointForm form);
  Future<rust.DeleteReportView> deleteEndpoint(String id, rust.TargetView? rebindTo);
  Future<void> moveEndpoint(String id, int index);

  // The look (LF-33): bragi-core's themes, shared with the desktop and terminal apps.
  Future<List<rust.ThemeRowView>> themes();
  List<rust.RoleInfoView> themeRoles();
  rust.ThemePrefView themePreference();
  Future<rust.ThemePrefView> setThemePreference(String theme, String mode);
  rust.PaletteView currentPalette(bool? systemDark);
  rust.PaletteView themePalette(String name, bool dark);
  Future<String> themeEditable(String name);
  Future<rust.PaletteView> themeApply(String name, bool dark, List<rust.RoleEdit> edits);
  Future<String> themeDuplicate(String name, String? asName);
  Future<void> themeRename(String from, String to);
  Future<void> themeDelete(String name);
  Future<void> themeReset(String name);
  Future<String> themeExportText(String name);
  Future<String> themeImportText(String text, String? asName);
  Future<List<String>> themeWarnings(String name, bool dark);
  Future<rust.TestResultView> testEndpoint(String id);
  Future<rust.TestResultView> testEndpointAddress(String baseUrl, String? apiKey);
  Future<List<String>> endpointModels(String id);
  Future<List<rust.LinkView>> links();
  Future<String> editLink(String old, String address);
  Future<void> removeLink(String address);
  Future<List<rust.TargetView>> targets();
  Future<rust.ChatStarted> sendTo(String? conversation, String prompt, String? model, List<rust.AttachmentView> files, rust.TargetView? target);
  Future<List<rust.SessionView>> sessions(int offset, int limit);
  Future<void> bindSession(String conversation, rust.TargetView target);
  Future<List<rust.FolderView>> folders();
  Future<rust.FolderView> createFolder(String name, String? parent);
  Future<void> renameFolder(String id, String name);
  Future<void> moveFolder(String id, String? parent);
  Future<void> deleteFolder(String id);
  Future<void> setSessionFolder(String conversation, String? folder);

  // Models on the user's Ymir pool (LF-30).
  Future<List<rust.NodeModelsView>> modelsOverview();
  /// Starts a Hub search with a fit per node; stream it with [followModels].
  Future<String> modelsSearch(String query);
  /// Starts a pull to `target` (`auto`, `all` or a node's peer id); stream it with [followModels]; [cancel] stops it.
  Future<String> modelsPull(String repo, String? model, String target, bool serve);
  Stream<rust.ModelsEventView> followModels(String runId);
  Future<BigInt> modelsRemove(String peer, String id, bool confirmed);
}

class RustBragiApi implements BragiApi {
  const RustBragiApi();

  @override
  Stream<rust.StateView> watchState() => rust.watchState();
  @override
  Future<String> pairOffer() => rust.pairOffer();
  @override
  Future<void> pairConfirm(bool matches) => rust.pairConfirm(matches: matches);
  @override
  Future<void> pairCancel() => rust.pairCancel();
  @override
  Future<rust.LinkCheckView> pairAccept(String link) => rust.pairAccept(link: link);
  @override
  bool isNodeAddress(String text) => rust.isNodeAddress(text: text);
  @override
  String addNode(String address) => rust.addNode(address: address);
  @override
  String thinkingLabel(int elapsedMs, bool done) => rust.thinkingLabel(elapsedMs: BigInt.from(elapsedMs), done: done);
  @override
  List<String> peers() => rust.peers();
  @override
  void dismissNodeSearch() => rust.dismissNodeSearch();
  @override
  Future<void> pairAcceptConfirm(bool matches) => rust.pairAcceptConfirm(matches: matches);
  @override
  Future<rust.ChatStarted> send(String? conversation, String prompt, String? model) =>
      rust.send(conversation: conversation, prompt: prompt, model: model);
  @override
  Future<rust.ChatStarted> sendWith(String? conversation, String prompt, String? model, List<rust.AttachmentView> files) =>
      rust.sendWith(conversation: conversation, prompt: prompt, model: model, files: files);
  @override
  Future<rust.ChatStarted> sendAgent(String? conversation, String prompt, String? node, List<rust.AttachmentView> files) =>
      rust.sendAgent(conversation: conversation, prompt: prompt, node: node, files: files);
  @override
  rust.AttachmentView attachBytes(String name, List<int> bytes) => rust.attachBytes(name: name, bytes: bytes);
  @override
  bool approve(String requestId, int ask, String choice) => rust.approve(requestId: requestId, ask: BigInt.from(ask), choice: choice);
  @override
  Future<rust.ChatStarted> continueAnswer(String conversation, String? model) =>
      rust.continueAnswer(conversation: conversation, model: model);
  @override
  Stream<rust.ChatEventView> followChat(String requestId) => rust.followChat(requestId: requestId);
  @override
  bool cancel(String requestId) => rust.cancel(requestId: requestId);
  @override
  Future<List<rust.SummaryView>> conversations(int offset, int limit) => rust.conversations(offset: offset, limit: limit);
  @override
  Future<rust.ConversationView> conversation(String id) => rust.conversation(id: id);
  @override
  Future<Uint8List> fileBytes(String conversation, String file) => rust.fileBytes(conversation: conversation, file: file);
  @override
  Future<String> saveFile(String conversation, String file, String dest) => rust.saveFile(conversation: conversation, file: file, dest: dest);
  @override
  Future<rust.DeletedView> deleteMessage(String conversation, String id, bool withReply) =>
      rust.deleteMessage(conversation: conversation, id: id, withReply: withReply);
  @override
  Future<rust.DeletedView> deleteConversations(List<String> ids) => rust.deleteConversations(ids: ids);
  @override
  Future<void> undoDelete(String undo) => rust.undoDelete(undo: undo);
  @override
  Future<int> clearHistory() => rust.clearHistory();
  @override
  Future<List<String>> react(String conversation, String message, String emoji) =>
      rust.react(conversation: conversation, message: message, emoji: emoji);
  @override
  List<String> quickReactions() => rust.quickReactions();
  @override
  Future<void> paused() => rust.paused();
  @override
  Future<void> resumed() => rust.resumed();
  @override
  String markdownJson(String text, bool streaming) => rust.markdownJson(text: text, streaming: streaming);

  @override
  Future<List<rust.EndpointView>> endpoints() => rust.endpoints();
  @override
  Future<rust.EndpointView> addEndpoint(rust.EndpointForm form) => rust.addEndpoint(formIn: form);
  @override
  Future<rust.EndpointView> editEndpoint(String id, rust.EndpointForm form) => rust.editEndpoint(id: id, formIn: form);
  @override
  Future<rust.DeleteReportView> deleteEndpoint(String id, rust.TargetView? rebindTo) => rust.deleteEndpoint(id: id, rebindTo: rebindTo);
  @override
  Future<void> moveEndpoint(String id, int index) => rust.moveEndpoint(id: id, index: index);

  @override
  Future<List<rust.ThemeRowView>> themes() => rust.themes();
  @override
  List<rust.RoleInfoView> themeRoles() => rust.themeRoles();
  @override
  rust.ThemePrefView themePreference() => rust.themePreference();
  @override
  Future<rust.ThemePrefView> setThemePreference(String theme, String mode) => rust.setThemePreference(theme: theme, mode: mode);
  @override
  rust.PaletteView currentPalette(bool? systemDark) => rust.currentPalette(systemDark: systemDark);
  @override
  rust.PaletteView themePalette(String name, bool dark) => rust.themePalette(name: name, dark: dark);
  @override
  Future<String> themeEditable(String name) => rust.themeEditable(name: name);
  @override
  Future<rust.PaletteView> themeApply(String name, bool dark, List<rust.RoleEdit> edits) => rust.themeApply(name: name, dark: dark, edits: edits);
  @override
  Future<String> themeDuplicate(String name, String? asName) => rust.themeDuplicate(name: name, asName: asName);
  @override
  Future<void> themeRename(String from, String to) => rust.themeRename(from: from, to: to);
  @override
  Future<void> themeDelete(String name) => rust.themeDelete(name: name);
  @override
  Future<void> themeReset(String name) => rust.themeReset(name: name);
  @override
  Future<String> themeExportText(String name) => rust.themeExportText(name: name);
  @override
  Future<String> themeImportText(String text, String? asName) => rust.themeImportText(text: text, asName: asName);
  @override
  Future<List<String>> themeWarnings(String name, bool dark) => rust.themeWarnings(name: name, dark: dark);
  @override
  Future<rust.TestResultView> testEndpoint(String id) => rust.testEndpoint(id: id);
  @override
  Future<rust.TestResultView> testEndpointAddress(String baseUrl, String? apiKey) => rust.testEndpointAddress(baseUrl: baseUrl, apiKey: apiKey);
  @override
  Future<List<String>> endpointModels(String id) => rust.endpointModels(id: id);
  @override
  Future<List<rust.LinkView>> links() => rust.links();
  @override
  Future<String> editLink(String old, String address) => rust.editLink(old: old, new_: address);
  @override
  Future<void> removeLink(String address) => rust.removeLink(address: address);
  @override
  Future<List<rust.TargetView>> targets() => rust.targets();
  @override
  Future<rust.ChatStarted> sendTo(String? conversation, String prompt, String? model, List<rust.AttachmentView> files, rust.TargetView? target) =>
      rust.sendTo(conversation: conversation, prompt: prompt, model: model, files: files, target: target);
  @override
  Future<List<rust.SessionView>> sessions(int offset, int limit) => rust.sessions(offset: offset, limit: limit);
  @override
  Future<void> bindSession(String conversation, rust.TargetView target) => rust.bindSession(conversation: conversation, target: target);
  @override
  Future<List<rust.FolderView>> folders() => rust.folders();
  @override
  Future<rust.FolderView> createFolder(String name, String? parent) => rust.createFolder(name: name, parent: parent);
  @override
  Future<void> renameFolder(String id, String name) => rust.renameFolder(id: id, name: name);
  @override
  Future<void> moveFolder(String id, String? parent) => rust.moveFolder(id: id, parent: parent);
  @override
  Future<void> deleteFolder(String id) => rust.deleteFolder(id: id);
  @override
  Future<void> setSessionFolder(String conversation, String? folder) => rust.setSessionFolder(conversation: conversation, folder: folder);

  @override
  Future<List<rust.NodeModelsView>> modelsOverview() => rust.modelsOverview();
  @override
  Future<String> modelsSearch(String query) async => rust.modelsSearch(query: query);
  @override
  Future<String> modelsPull(String repo, String? model, String target, bool serve) async =>
      rust.modelsPull(repo: repo, model: model, target: target, serve: serve);
  @override
  Stream<rust.ModelsEventView> followModels(String runId) => rust.followModels(runId: runId);
  @override
  Future<BigInt> modelsRemove(String peer, String id, bool confirmed) => rust.modelsRemove(peer: peer, id: id, confirmed: confirmed);
}
