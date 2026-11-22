// The seam between the widgets and bragi-core: [RustBragiApi] calls the bridge; tests may
// pass their own. The data types are the bridge's (lib/src/rust/api/bragi.dart).
import 'rust/api/bragi.dart' as rust;

export 'rust/api/bragi.dart'
    show
        AttachmentView,
        ChatEventView,
        ChatStarted,
        ConversationView,
        EndpointTestView,
        EndpointView,
        FolderView,
        LinkCheckView,
        LinkView,
        MessageView,
        ModelView,
        PairView,
        RouteView,
        SessionView,
        StateView,
        SummaryView,
        TreeRowView;

abstract class BragiApi {
  Stream<rust.StateView> watchState();
  Future<String> pairOffer();
  Future<void> pairConfirm(bool matches);
  Future<void> pairCancel();
  Future<rust.LinkCheckView> pairAccept(String link);
  bool isNodeAddress(String text);
  String addNode(String address);
  List<String> peers();
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
  Future<void> paused();
  Future<void> resumed();
  // Sessions and folders.
  Future<rust.SessionView> createSession(String name, String? endpointId, String? model, String? folder);
  Future<void> renameSession(String id, String name);
  Future<void> moveSession(String id, String? folder);
  Future<void> setSessionModel(String id, String? model);
  Future<void> setSessionEndpoint(String id, String? endpointId);
  Future<void> switchSession(String id);
  Future<void> deleteSession(String id);
  Future<rust.FolderView> createFolder(String name, String? parent);
  Future<void> renameFolder(String id, String name);
  Future<void> moveFolder(String id, String? parent);
  Future<void> setFolderCollapsed(String id, bool collapsed);
  Future<void> deleteFolder(String id);
  // Endpoints and the fallback.
  Future<rust.EndpointView> addEndpoint(String name, String url, String token);
  Future<rust.EndpointView> updateEndpoint(String id, String? name, String? url, String? token);
  Future<void> removeEndpoint(String id);
  Future<void> setFallback(String? id);
  Future<rust.EndpointTestView> testEndpoint(String? id, String url, String? token);
  Future<rust.EndpointView> editEndpoint(String id, String name, String url, String? token, String? model, bool fallback);
  Future<int> deleteEndpoint(String id, String rebind);

  // Weirdnet links (node addresses).
  Future<String> testLink(String address);
  Future<rust.LinkView> addLink(String name, String address);
  Future<void> renameLink(String address, String name);
  Future<rust.LinkView> editLink(String address, String newAddress);
  Future<int> deleteLink(String address);
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
  List<String> peers() => rust.peers();
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
  Future<void> paused() => rust.paused();
  @override
  Future<void> resumed() => rust.resumed();
  @override
  Future<rust.SessionView> createSession(String name, String? endpointId, String? model, String? folder) =>
      rust.createSession(name: name, endpointId: endpointId, model: model, folder: folder);
  @override
  Future<void> renameSession(String id, String name) => rust.renameSession(id: id, name: name);
  @override
  Future<void> moveSession(String id, String? folder) => rust.moveSession(id: id, folder: folder);
  @override
  Future<void> setSessionModel(String id, String? model) => rust.setSessionModel(id: id, model: model);
  @override
  Future<void> setSessionEndpoint(String id, String? endpointId) => rust.setSessionEndpoint(id: id, endpointId: endpointId);
  @override
  Future<void> switchSession(String id) => rust.switchSession(id: id);
  @override
  Future<void> deleteSession(String id) => rust.deleteSession(id: id);
  @override
  Future<rust.FolderView> createFolder(String name, String? parent) => rust.createFolder(name: name, parent: parent);
  @override
  Future<void> renameFolder(String id, String name) => rust.renameFolder(id: id, name: name);
  @override
  Future<void> moveFolder(String id, String? parent) => rust.moveFolder(id: id, parent: parent);
  @override
  Future<void> setFolderCollapsed(String id, bool collapsed) => rust.setFolderCollapsed(id: id, collapsed: collapsed);
  @override
  Future<void> deleteFolder(String id) => rust.deleteFolder(id: id);
  @override
  Future<rust.EndpointView> addEndpoint(String name, String url, String token) => rust.addEndpoint(name: name, url: url, token: token);
  @override
  Future<rust.EndpointView> updateEndpoint(String id, String? name, String? url, String? token) =>
      rust.updateEndpoint(id: id, name: name, url: url, token: token);
  @override
  Future<void> removeEndpoint(String id) => rust.removeEndpoint(id: id);
  @override
  Future<void> setFallback(String? id) => rust.setFallback(id: id);
  @override
  Future<rust.EndpointTestView> testEndpoint(String? id, String url, String? token) => rust.testEndpoint(id: id, url: url, token: token);
  @override
  Future<rust.EndpointView> editEndpoint(String id, String name, String url, String? token, String? model, bool fallback) =>
      rust.editEndpoint(id: id, name: name, url: url, token: token, model: model, fallback: fallback);
  @override
  Future<int> deleteEndpoint(String id, String rebind) => rust.deleteEndpoint(id: id, rebind: rebind);
  @override
  Future<String> testLink(String address) => rust.testLink(address: address);
  @override
  Future<rust.LinkView> addLink(String name, String address) => rust.addLink(name: name, address: address);
  @override
  Future<void> renameLink(String address, String name) => rust.renameLink(address: address, name: name);
  @override
  Future<rust.LinkView> editLink(String address, String newAddress) => rust.editLink(address: address, newAddress: newAddress);
  @override
  Future<int> deleteLink(String address) => rust.deleteLink(address: address);
}
