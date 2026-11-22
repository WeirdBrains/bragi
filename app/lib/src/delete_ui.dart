// Deleting history: the message menu, the confirm dialogs, the Undo snackbar, and Clear all
// history for Settings. Every delete is one core call (bragi-core's history::forget); this
// only asks first. Sessions and folders reuse these.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'theme.dart';

/// The word Clear all history wants typed.
const clearWord = 'delete';

/// Offers Undo for the delete that just happened, for as long as the core keeps it.
void showUndo(BuildContext context, BragiController c, String what) {
  final d = c.deleted;
  if (d == null) return;
  final messenger = ScaffoldMessenger.of(context);
  messenger.hideCurrentSnackBar();
  messenger.showSnackBar(
    SnackBar(
      key: const Key('undo-bar'),
      content: Text(what),
      // A little inside the core's window, so Undo never arrives too late.
      duration: Duration(milliseconds: (d.undoMs - 1500).clamp(2000, 60000)),
      action: SnackBarAction(key: const Key('undo'), label: 'Undo', onPressed: c.undoDelete),
    ),
  );
}

/// The long-press menu on a message.
Future<void> messageMenu(BuildContext context, BragiController c, MessageView m) async {
  final picked = await showModalBottomSheet<bool>(
    context: context,
    builder: (sheet) => SafeArea(
      child: ListTile(
        key: const Key('menu-delete'),
        leading: const Icon(Icons.delete_outline, color: BragiColors.error),
        title: const Text('Delete'),
        onTap: () => Navigator.of(sheet).pop(true),
      ),
    ),
  );
  if (picked == true && context.mounted) await confirmDeleteMessage(context, c, m);
}

/// Asks before deleting a message; a question with an answer after it asks about that too.
Future<void> confirmDeleteMessage(BuildContext context, BragiController c, MessageView m) async {
  final withReply = await showDialog<bool>(
    context: context,
    builder: (d) => AlertDialog(
      title: const Text('Delete this message?'),
      content: Text(
        m.hasReply
            ? 'Delete the answer that followed too? It goes from all your linked devices and is not sent to a model again.'
            : 'It goes from all your linked devices and is not sent to a model again.',
      ),
      actions: [
        TextButton(key: const Key('delete-cancel'), onPressed: () => Navigator.of(d).pop(), child: const Text('Cancel')),
        if (m.hasReply)
          TextButton(key: const Key('delete-only'), onPressed: () => Navigator.of(d).pop(false), child: const Text('Only this')),
        FilledButton(
          key: Key(m.hasReply ? 'delete-with-reply' : 'delete-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: () => Navigator.of(d).pop(m.hasReply),
          child: Text(m.hasReply ? 'With the answer' : 'Delete'),
        ),
      ],
    ),
  );
  if (withReply == null || !context.mounted) return;
  if (await c.deleteMessage(m, withReply: withReply) && context.mounted) {
    showUndo(context, c, withReply ? 'Deleted the message and its answer.' : 'Deleted the message.');
  }
}

/// Asks before deleting a conversation from the history list.
Future<void> confirmDeleteConversation(BuildContext context, BragiController c, SummaryView s) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (d) => AlertDialog(
      title: const Text('Delete this conversation?'),
      content: Text('“${s.title}” goes from all your linked devices.'),
      actions: [
        TextButton(key: const Key('delete-cancel'), onPressed: () => Navigator.of(d).pop(false), child: const Text('Cancel')),
        FilledButton(
          key: const Key('delete-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: () => Navigator.of(d).pop(true),
          child: const Text('Delete'),
        ),
      ],
    ),
  );
  if (ok != true || !context.mounted) return;
  if (await c.deleteConversation(s.id) && context.mounted) showUndo(context, c, 'Deleted “${s.title}”.');
}

/// Clear all history, behind a typed confirm: there is no undo.
class ClearHistoryTile extends StatelessWidget {
  const ClearHistoryTile({super.key, required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      key: const Key('clear-history'),
      leading: const Icon(Icons.delete_forever_outlined, color: BragiColors.error),
      title: const Text('Clear all history'),
      subtitle: const Text('Every conversation, on all your linked devices. This can’t be undone.'),
      onTap: () async {
        final ok = await showDialog<bool>(context: context, builder: (_) => const _ClearDialog());
        if (ok == true && context.mounted) {
          await controller.clearHistory();
          if (context.mounted && controller.notice != null) {
            ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(controller.notice!)));
          }
        }
      },
    );
  }
}

class _ClearDialog extends StatefulWidget {
  const _ClearDialog();

  @override
  State<_ClearDialog> createState() => _ClearDialogState();
}

class _ClearDialogState extends State<_ClearDialog> {
  final _typed = TextEditingController();

  @override
  void dispose() {
    _typed.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ready = _typed.text.trim().toLowerCase() == clearWord;
    return AlertDialog(
      title: const Text('Clear all history?'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Every conversation goes, on all your linked devices, with no undo. Type “$clearWord” to go ahead.'),
          const SizedBox(height: 10),
          TextField(
            key: const Key('clear-confirm-text'),
            controller: _typed,
            autofocus: true,
            onChanged: (_) => setState(() {}),
            decoration: const InputDecoration(hintText: clearWord, isDense: true, border: OutlineInputBorder()),
          ),
        ],
      ),
      actions: [
        TextButton(key: const Key('clear-cancel'), onPressed: () => Navigator.of(context).pop(false), child: const Text('Cancel')),
        FilledButton(
          key: const Key('clear-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: ready ? () => Navigator.of(context).pop(true) : null,
          child: const Text('Clear all history'),
        ),
      ],
    );
  }
}

/// Settings. For now only history; the endpoints and sessions settings join it later.
class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        children: [
          const Padding(
            padding: EdgeInsets.fromLTRB(16, 16, 16, 4),
            child: Text('HISTORY', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
          ),
          ClearHistoryTile(controller: controller),
        ],
      ),
    );
  }
}
