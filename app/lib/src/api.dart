// The seam between the widgets and bragi-core: [RustBragiApi] calls the bridge; tests may
// pass their own. The data types are the bridge's (lib/src/rust/api/bragi.dart).
import 'rust/api/bragi.dart' as rust;

export 'rust/api/bragi.dart'
    show
        AttachmentView,
        ChatEventView,
        ChatStarted,
        ConversationView,
        DeletedView,
        LinkCheckView,
        MessageView,
        ModelView,
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
  Future<rust.DeletedView> deleteMessage(String conversation, String id, bool withReply) =>
      rust.deleteMessage(conversation: conversation, id: id, withReply: withReply);
  @override
  Future<rust.DeletedView> deleteConversations(List<String> ids) => rust.deleteConversations(ids: ids);
  @override
  Future<void> undoDelete(String undo) => rust.undoDelete(undo: undo);
  @override
  Future<int> clearHistory() => rust.clearHistory();
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
}
