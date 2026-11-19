// The seam between the widgets and bragi-core: [RustBragiApi] calls the bridge; tests may
// pass their own. The data types are the bridge's (lib/src/rust/api/bragi.dart).
import 'rust/api/bragi.dart' as rust;

export 'rust/api/bragi.dart'
    show
        ChatEventView,
        ChatStarted,
        ConversationView,
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
  Future<void> pairAcceptConfirm(bool matches);
  Future<rust.ChatStarted> send(String? conversation, String prompt, String? model);
  Future<rust.ChatStarted> continueAnswer(String conversation, String? model);
  Stream<rust.ChatEventView> followChat(String requestId);
  bool cancel(String requestId);
  Future<List<rust.SummaryView>> conversations(int offset, int limit);
  Future<rust.ConversationView> conversation(String id);
  Future<void> paused();
  Future<void> resumed();
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
  Future<void> pairAcceptConfirm(bool matches) => rust.pairAcceptConfirm(matches: matches);
  @override
  Future<rust.ChatStarted> send(String? conversation, String prompt, String? model) =>
      rust.send(conversation: conversation, prompt: prompt, model: model);
  @override
  Future<rust.ChatStarted> continueAnswer(String conversation, String? model) =>
      rust.continueAnswer(conversation: conversation, model: model);
  @override
  Stream<rust.ChatEventView> followChat(String requestId) => rust.followChat(requestId: requestId);
  @override
  bool cancel(String requestId) => rust.cancel(requestId: requestId);
  @override
  Future<List<rust.SummaryView>> conversations(int offset, int limit) =>
      rust.conversations(offset: offset, limit: limit);
  @override
  Future<rust.ConversationView> conversation(String id) => rust.conversation(id: id);
  @override
  Future<void> paused() => rust.paused();
  @override
  Future<void> resumed() => rust.resumed();
}
