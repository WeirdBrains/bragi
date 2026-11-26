// The seam between the widgets and bragi-core: [RustBragiApi] calls the bridge; tests may
// pass their own. The data types are the bridge's (lib/src/rust/api/bragi.dart).
import 'dart:typed_data';

import 'rust/api/bragi.dart' as rust;
import 'rust/api/video.dart' as rv;

export 'rust/api/video.dart'
    show VideoFrameIn, VideoFramePlanView, VideoFrameView, VideoInfoView, VideoLimitsView, VideoProbeView;
export 'rust/api/bragi.dart'
    show
        NodeTrainingView,
        RolloutVersionView,
        RolloutView,
        TrainingJobView,
        TrainingOverviewView,
        TrainingRunView,
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
        PoolView,
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
        PendingUploadView,
        UploadCopyView,
        UploadPreviewView,
        UploadSpecView,
        UploadedResultView,
        StateView,
        SummaryView,
        WakeView,
        WakeableView;

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
  Future<rust.ChatStarted> send(
    String? conversation,
    String prompt,
    String? model,
  );
  Future<rust.ChatStarted> sendWith(
    String? conversation,
    String prompt,
    String? model,
    List<rust.AttachmentView> files,
  );
  Future<rust.ChatStarted> sendAgent(
    String? conversation,
    String prompt,
    String? node,
    List<rust.AttachmentView> files,
  );
  rust.AttachmentView attachBytes(String name, List<int> bytes);

  // Video (LF-43, V3): the core plans the frames, a phone's platform decoder makes them, the core
  // attaches them; a computer reads the clip with ffmpeg through the core.
  Future<rv.VideoFramePlanView> videoFramePlan(String name, rv.VideoProbeView probe, String? model, rust.TargetView? target, List<String> history);
  Future<rust.AttachmentView> attachVideoFrames(String name, String? path, rv.VideoProbeView probe, List<rv.VideoFrameIn> frames, String? model, rust.TargetView? target, List<String> history);
  Future<rust.AttachmentView> attachVideoFile(String path, String? model, rust.TargetView? target, List<String> history);
  String? videoFileUnavailable();
  rv.VideoInfoView videoInfo(String name, String json);
  rv.VideoInfoView? videoAttachment(String conversation, String file);
  Future<rv.VideoLimitsView> videoLimits(String? model, rust.TargetView? target);
  Future<rv.VideoLimitsView> setVideoLimits(String? model, String limits, rust.TargetView? target);
  bool approve(String requestId, int ask, String choice);
  Future<rust.ChatStarted> continueAnswer(String conversation, String? model);
  Stream<rust.ChatEventView> followChat(String requestId);
  bool cancel(String requestId);
  Future<List<rust.SummaryView>> conversations(int offset, int limit);
  Future<rust.ConversationView> conversation(String id);
  Future<Uint8List> fileBytes(String conversation, String file);
  Future<String> saveFile(String conversation, String file, String dest);
  Future<rust.DeletedView> deleteMessage(
    String conversation,
    String id,
    bool withReply,
  );
  Future<rust.DeletedView> deleteConversations(List<String> ids);
  Future<void> setTraining(String id, bool on);
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
  Future<rust.DeleteReportView> deleteEndpoint(
    String id,
    rust.TargetView? rebindTo,
  );
  Future<void> moveEndpoint(String id, int index);

  // The look (LF-33): bragi-core's themes, shared with the desktop and terminal apps.
  Future<List<rust.ThemeRowView>> themes();
  List<rust.RoleInfoView> themeRoles();
  rust.ThemePrefView themePreference();
  Future<rust.ThemePrefView> setThemePreference(String theme, String mode);
  rust.PaletteView currentPalette(bool? systemDark);
  rust.PaletteView themePalette(String name, bool dark);
  Future<String> themeEditable(String name);
  Future<rust.PaletteView> themeApply(
    String name,
    bool dark,
    List<rust.RoleEdit> edits,
  );
  Future<String> themeDuplicate(String name, String? asName);
  Future<void> themeRename(String from, String to);
  Future<void> themeDelete(String name);
  Future<void> themeReset(String name);
  Future<String> themeExportText(String name);
  Future<String> themeImportText(String text, String? asName);
  Future<List<String>> themeWarnings(String name, bool dark);
  Future<rust.TestResultView> testEndpoint(String id);
  Future<rust.TestResultView> testEndpointAddress(
    String baseUrl,
    String? apiKey,
  );
  Future<List<String>> endpointModels(String id);
  Future<List<rust.LinkView>> links();
  Future<String> editLink(String old, String address);
  Future<void> removeLink(String address);
  Future<List<rust.PoolView>> pools();
  Future<rust.PoolView?> activePool();
  Future<String> addPool();
  Future<rust.PoolView> switchPool(String id);
  Future<rust.PoolView> renamePool(String id, String name);
  Future<void> removePool(String id);
  bool sidebarCollapsed();
  void setSidebarCollapsed(bool collapsed);
  // Push wake-ups (B4).
  rust.WakeView wakeStatus();
  rust.WakeView setWakeSettings(bool push, bool relayWake);
  String relayWakeCost();
  Future<List<rust.TargetView>> targets();
  Future<rust.ChatStarted> sendTo(
    String? conversation,
    String prompt,
    String? model,
    List<rust.AttachmentView> files,
    rust.TargetView? target,
  );
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

  /// The training view: runs and jobs per node (Ymir 0.16.0) and the inference nodes' rollouts.
  Future<rust.TrainingOverviewView> trainingOverview();

  /// Starts a Hub search with a fit per node; stream it with [followModels].
  Future<String> modelsSearch(String query);

  /// Starts a pull to `target` (`auto`, `all` or a node's peer id); stream it with [followModels]; [cancel] stops it.
  Future<String> modelsPull(
    String repo,
    String? model,
    String target,
    bool serve,
  );
  Stream<rust.ModelsEventView> followModels(String runId);
  Future<BigInt> modelsRemove(String peer, String id, bool confirmed);

  // Uploads of a model file from this device to a model node (LF-30 upload).
  /// The inference nodes (peer ids) an upload's rollout can go to.
  List<String> modelsInferenceNodes();

  /// The share presets for a rollout, in percent.
  List<int> uploadPercentPresets();

  /// Reads and hashes a file on this device: what it is, before anything is sent.
  Future<rust.UploadPreviewView> modelsInspectUpload(String path);

  /// Starts an upload; stream it with [followModels] (`progress`, then `uploaded`, or `failed`).
  Future<String> modelsUpload(rust.UploadSpecView spec);

  /// The same upload as a `ymirctl` command for a computer that holds the file at [file].
  Future<String> uploadCommand(rust.UploadSpecView spec, String file, List<rust.ModelNodeView> labels);
  Future<List<rust.PendingUploadView>> modelsPendingUploads();
  Future<void> modelsForgetUpload(String path, String peer);
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
  Future<rust.LinkCheckView> pairAccept(String link) =>
      rust.pairAccept(link: link);
  @override
  bool isNodeAddress(String text) => rust.isNodeAddress(text: text);
  @override
  String addNode(String address) => rust.addNode(address: address);
  @override
  String thinkingLabel(int elapsedMs, bool done) =>
      rust.thinkingLabel(elapsedMs: BigInt.from(elapsedMs), done: done);
  @override
  List<String> peers() => rust.peers();
  @override
  void dismissNodeSearch() => rust.dismissNodeSearch();
  @override
  Future<void> pairAcceptConfirm(bool matches) =>
      rust.pairAcceptConfirm(matches: matches);
  @override
  Future<rust.ChatStarted> send(
    String? conversation,
    String prompt,
    String? model,
  ) => rust.send(conversation: conversation, prompt: prompt, model: model);
  @override
  Future<rust.ChatStarted> sendWith(
    String? conversation,
    String prompt,
    String? model,
    List<rust.AttachmentView> files,
  ) => rust.sendWith(
    conversation: conversation,
    prompt: prompt,
    model: model,
    files: files,
  );
  @override
  Future<rust.ChatStarted> sendAgent(
    String? conversation,
    String prompt,
    String? node,
    List<rust.AttachmentView> files,
  ) => rust.sendAgent(
    conversation: conversation,
    prompt: prompt,
    node: node,
    files: files,
  );
  @override
  rust.AttachmentView attachBytes(String name, List<int> bytes) =>
      rust.attachBytes(name: name, bytes: bytes);
  @override
  Future<rv.VideoFramePlanView> videoFramePlan(String name, rv.VideoProbeView probe, String? model, rust.TargetView? target, List<String> history) =>
      rv.videoFramePlan(name: name, probe: probe, model: model, target: target, history: history);
  @override
  Future<rust.AttachmentView> attachVideoFrames(String name, String? path, rv.VideoProbeView probe, List<rv.VideoFrameIn> frames, String? model, rust.TargetView? target, List<String> history) =>
      rv.attachVideoFrames(name: name, path: path, probe: probe, frames: frames, model: model, target: target, history: history);
  @override
  Future<rust.AttachmentView> attachVideoFile(String path, String? model, rust.TargetView? target, List<String> history) =>
      rv.attachVideoFile(path: path, model: model, target: target, history: history);
  @override
  String? videoFileUnavailable() => rv.videoFileUnavailable();
  @override
  rv.VideoInfoView videoInfo(String name, String json) => rv.videoInfo(name: name, json: json);
  @override
  rv.VideoInfoView? videoAttachment(String conversation, String file) => rv.videoAttachment(conversation: conversation, file: file);
  @override
  Future<rv.VideoLimitsView> videoLimits(String? model, rust.TargetView? target) => rv.videoLimits(model: model, target: target);
  @override
  Future<rv.VideoLimitsView> setVideoLimits(String? model, String limits, rust.TargetView? target) =>
      rv.setVideoLimits(model: model, limits: limits, target: target);
  @override
  bool approve(String requestId, int ask, String choice) =>
      rust.approve(requestId: requestId, ask: BigInt.from(ask), choice: choice);
  @override
  Future<rust.ChatStarted> continueAnswer(String conversation, String? model) =>
      rust.continueAnswer(conversation: conversation, model: model);
  @override
  Stream<rust.ChatEventView> followChat(String requestId) =>
      rust.followChat(requestId: requestId);
  @override
  bool cancel(String requestId) => rust.cancel(requestId: requestId);
  @override
  Future<List<rust.SummaryView>> conversations(int offset, int limit) =>
      rust.conversations(offset: offset, limit: limit);
  @override
  Future<rust.ConversationView> conversation(String id) =>
      rust.conversation(id: id);
  @override
  Future<Uint8List> fileBytes(String conversation, String file) =>
      rust.fileBytes(conversation: conversation, file: file);
  @override
  Future<String> saveFile(String conversation, String file, String dest) =>
      rust.saveFile(conversation: conversation, file: file, dest: dest);
  @override
  Future<rust.DeletedView> deleteMessage(
    String conversation,
    String id,
    bool withReply,
  ) => rust.deleteMessage(
    conversation: conversation,
    id: id,
    withReply: withReply,
  );
  @override
  Future<rust.DeletedView> deleteConversations(List<String> ids) =>
      rust.deleteConversations(ids: ids);
  @override
  Future<void> setTraining(String id, bool on) => rust.setTraining(id: id, marked: on);
  @override
  Future<void> undoDelete(String undo) => rust.undoDelete(undo: undo);
  @override
  Future<int> clearHistory() => rust.clearHistory();
  @override
  Future<List<String>> react(
    String conversation,
    String message,
    String emoji,
  ) => rust.react(conversation: conversation, message: message, emoji: emoji);
  @override
  List<String> quickReactions() => rust.quickReactions();
  @override
  Future<void> paused() => rust.paused();
  @override
  Future<void> resumed() => rust.resumed();
  @override
  String markdownJson(String text, bool streaming) =>
      rust.markdownJson(text: text, streaming: streaming);

  @override
  Future<List<rust.EndpointView>> endpoints() => rust.endpoints();
  @override
  Future<rust.EndpointView> addEndpoint(rust.EndpointForm form) =>
      rust.addEndpoint(formIn: form);
  @override
  Future<rust.EndpointView> editEndpoint(String id, rust.EndpointForm form) =>
      rust.editEndpoint(id: id, formIn: form);
  @override
  Future<rust.DeleteReportView> deleteEndpoint(
    String id,
    rust.TargetView? rebindTo,
  ) => rust.deleteEndpoint(id: id, rebindTo: rebindTo);
  @override
  Future<void> moveEndpoint(String id, int index) =>
      rust.moveEndpoint(id: id, index: index);

  @override
  Future<List<rust.ThemeRowView>> themes() => rust.themes();
  @override
  List<rust.RoleInfoView> themeRoles() => rust.themeRoles();
  @override
  rust.ThemePrefView themePreference() => rust.themePreference();
  @override
  Future<rust.ThemePrefView> setThemePreference(String theme, String mode) =>
      rust.setThemePreference(theme: theme, mode: mode);
  @override
  rust.PaletteView currentPalette(bool? systemDark) =>
      rust.currentPalette(systemDark: systemDark);
  @override
  rust.PaletteView themePalette(String name, bool dark) =>
      rust.themePalette(name: name, dark: dark);
  @override
  Future<String> themeEditable(String name) => rust.themeEditable(name: name);
  @override
  Future<rust.PaletteView> themeApply(
    String name,
    bool dark,
    List<rust.RoleEdit> edits,
  ) => rust.themeApply(name: name, dark: dark, edits: edits);
  @override
  Future<String> themeDuplicate(String name, String? asName) =>
      rust.themeDuplicate(name: name, asName: asName);
  @override
  Future<void> themeRename(String from, String to) =>
      rust.themeRename(from: from, to: to);
  @override
  Future<void> themeDelete(String name) => rust.themeDelete(name: name);
  @override
  Future<void> themeReset(String name) => rust.themeReset(name: name);
  @override
  Future<String> themeExportText(String name) =>
      rust.themeExportText(name: name);
  @override
  Future<String> themeImportText(String text, String? asName) =>
      rust.themeImportText(text: text, asName: asName);
  @override
  Future<List<String>> themeWarnings(String name, bool dark) =>
      rust.themeWarnings(name: name, dark: dark);
  @override
  Future<rust.TestResultView> testEndpoint(String id) =>
      rust.testEndpoint(id: id);
  @override
  Future<rust.TestResultView> testEndpointAddress(
    String baseUrl,
    String? apiKey,
  ) => rust.testEndpointAddress(baseUrl: baseUrl, apiKey: apiKey);
  @override
  Future<List<String>> endpointModels(String id) => rust.endpointModels(id: id);
  @override
  Future<List<rust.LinkView>> links() => rust.links();
  @override
  Future<String> editLink(String old, String address) =>
      rust.editLink(old: old, new_: address);
  @override
  Future<void> removeLink(String address) => rust.removeLink(address: address);
  @override
  Future<List<rust.PoolView>> pools() => rust.pools();
  @override
  Future<rust.PoolView?> activePool() => rust.activePool();
  @override
  Future<String> addPool() => rust.addPool();
  @override
  Future<rust.PoolView> switchPool(String id) => rust.switchPool(id: id);
  @override
  Future<rust.PoolView> renamePool(String id, String name) =>
      rust.renamePool(id: id, name: name);
  @override
  Future<void> removePool(String id) => rust.removePool(id: id);
  @override
  bool sidebarCollapsed() => rust.sidebarCollapsed();
  @override
  void setSidebarCollapsed(bool collapsed) =>
      rust.setSidebarCollapsed(collapsed: collapsed);
  @override
  rust.WakeView wakeStatus() => rust.wakeStatus();
  @override
  rust.WakeView setWakeSettings(bool push, bool relayWake) =>
      rust.setWakeSettings(push: push, relayWake: relayWake);
  @override
  String relayWakeCost() => rust.relayWakeCost();
  @override
  Future<List<rust.TargetView>> targets() => rust.targets();
  @override
  Future<rust.ChatStarted> sendTo(
    String? conversation,
    String prompt,
    String? model,
    List<rust.AttachmentView> files,
    rust.TargetView? target,
  ) => rust.sendTo(
    conversation: conversation,
    prompt: prompt,
    model: model,
    files: files,
    target: target,
  );
  @override
  Future<List<rust.SessionView>> sessions(int offset, int limit) =>
      rust.sessions(offset: offset, limit: limit);
  @override
  Future<void> bindSession(String conversation, rust.TargetView target) =>
      rust.bindSession(conversation: conversation, target: target);
  @override
  Future<List<rust.FolderView>> folders() => rust.folders();
  @override
  Future<rust.FolderView> createFolder(String name, String? parent) =>
      rust.createFolder(name: name, parent: parent);
  @override
  Future<void> renameFolder(String id, String name) =>
      rust.renameFolder(id: id, name: name);
  @override
  Future<void> moveFolder(String id, String? parent) =>
      rust.moveFolder(id: id, parent: parent);
  @override
  Future<void> deleteFolder(String id) => rust.deleteFolder(id: id);
  @override
  Future<void> setSessionFolder(String conversation, String? folder) =>
      rust.setSessionFolder(conversation: conversation, folder: folder);

  @override
  Future<List<rust.NodeModelsView>> modelsOverview() => rust.modelsOverview();
  @override
  Future<rust.TrainingOverviewView> trainingOverview() => rust.trainingOverview();
  @override
  Future<String> modelsSearch(String query) async =>
      rust.modelsSearch(query: query);
  @override
  Future<String> modelsPull(
    String repo,
    String? model,
    String target,
    bool serve,
  ) async =>
      rust.modelsPull(repo: repo, model: model, target: target, serve: serve);
  @override
  Stream<rust.ModelsEventView> followModels(String runId) =>
      rust.followModels(runId: runId);
  @override
  Future<BigInt> modelsRemove(String peer, String id, bool confirmed) =>
      rust.modelsRemove(peer: peer, id: id, confirmed: confirmed);

  @override
  List<String> modelsInferenceNodes() => rust.modelsInferenceNodes();

  @override
  List<int> uploadPercentPresets() => rust.uploadPercentPresets().toList();

  @override
  Future<rust.UploadPreviewView> modelsInspectUpload(String path) => rust.modelsInspectUpload(path: path);

  @override
  Future<String> modelsUpload(rust.UploadSpecView spec) async => rust.modelsUpload(spec: spec);

  @override
  Future<String> uploadCommand(rust.UploadSpecView spec, String file, List<rust.ModelNodeView> labels) async =>
      rust.uploadCommand(spec: spec, file: file, labels: labels);

  @override
  Future<List<rust.PendingUploadView>> modelsPendingUploads() async => rust.modelsPendingUploads();

  @override
  Future<void> modelsForgetUpload(String path, String peer) async => rust.modelsForgetUpload(path: path, peer: peer);
}
