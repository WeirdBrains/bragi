// Sessions (LF-11): the target bar (what the open chat talks to), and the history drawer's
// nested folders. Folders and targets live in bragi-core; deleting a folder moves what it holds up
// a level and never deletes a conversation.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'delete_ui.dart';
import 'theme.dart';

/// Where the open chat goes: the Weirdnet pool or an endpoint. Pick another to rebind it.
class TargetBar extends StatelessWidget {
  const TargetBar({super.key, required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final unbound = c.needsTarget;
    return SizedBox(
      height: 44,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12),
        child: Row(
          children: [
            Icon(unbound ? Icons.warning_amber : Icons.hub_outlined, size: 16, color: unbound ? BragiColors.busy : BragiColors.textSecondary),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                unbound ? 'Its endpoint was deleted. Pick where to send this chat.' : c.targetLabel,
                key: const Key('target-label'),
                overflow: TextOverflow.ellipsis,
                style: TextStyle(fontSize: 13, color: unbound ? BragiColors.busy : BragiColors.textPrimary),
              ),
            ),
            PopupMenuButton<TargetView>(
              key: const Key('target-menu'),
              tooltip: 'Where this chat goes',
              icon: const Icon(Icons.expand_more),
              onSelected: c.pickTarget,
              itemBuilder: (_) => [
                for (var i = 0; i < c.targets.length; i++) PopupMenuItem(key: Key('target-$i'), value: c.targets[i], child: Text(c.targets[i].label)),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// The history drawer's list: folders (nested, collapsible) and the conversations in them.
class SessionsTree extends StatefulWidget {
  const SessionsTree({super.key, required this.controller});

  final BragiController controller;

  @override
  State<SessionsTree> createState() => _SessionsTreeState();
}

class _SessionsTreeState extends State<SessionsTree> {
  final _collapsed = <String>{};

  BragiController get c => widget.controller;

  List<Widget> _rows(BuildContext context, String? parent, int depth) {
    final out = <Widget>[];
    for (final f in c.folders.where((f) => f.parent == parent)) {
      final closed = _collapsed.contains(f.id);
      out.add(
        ListTile(
          key: Key('folder-${f.id}'),
          dense: true,
          contentPadding: EdgeInsets.only(left: 12.0 + depth * 16, right: 4),
          leading: Icon(closed ? Icons.folder : Icons.folder_open, size: 20),
          title: Text(f.name, overflow: TextOverflow.ellipsis),
          onTap: () => setState(() => closed ? _collapsed.remove(f.id) : _collapsed.add(f.id)),
          trailing: PopupMenuButton<String>(
            key: Key('folder-menu-${f.id}'),
            icon: const Icon(Icons.more_vert, size: 18),
            onSelected: (v) => switch (v) {
              'rename' => showFolderDialog(context, c, rename: f),
              'sub' => showFolderDialog(context, c, parent: f.id),
              'move' => pickFolder(context, c, title: 'Move “${f.name}” to', exclude: f.id).then((r) {
                if (r != null) c.moveFolder(f.id, r.folder);
              }),
              _ => confirmDeleteFolder(context, c, f),
            },
            itemBuilder: (_) => const [
              PopupMenuItem(key: Key('folder-rename'), value: 'rename', child: Text('Rename')),
              PopupMenuItem(key: Key('folder-sub'), value: 'sub', child: Text('New folder inside')),
              PopupMenuItem(key: Key('folder-move'), value: 'move', child: Text('Move…')),
              PopupMenuItem(key: Key('folder-delete'), value: 'delete', child: Text('Delete folder')),
            ],
          ),
        ),
      );
      if (!closed) out.addAll(_rows(context, f.id, depth + 1));
    }
    for (final s in c.sessions.where((s) => s.folder == parent)) {
      out.add(
        ListTile(
          key: Key('conv-${s.id}'),
          dense: true,
          contentPadding: EdgeInsets.only(left: 12.0 + depth * 16, right: 4),
          selected: c.current?.id == s.id,
          title: Text(s.title, overflow: TextOverflow.ellipsis),
          subtitle: Text(
            s.targetLabel,
            key: Key('conv-target-${s.id}'),
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: 11, color: s.unbound ? BragiColors.busy : BragiColors.textSecondary),
          ),
          trailing: Wrap(
            children: [
              IconButton(
                key: Key('move-conv-${s.id}'),
                tooltip: 'Move to a folder',
                visualDensity: VisualDensity.compact,
                icon: const Icon(Icons.drive_file_move_outline, size: 18),
                onPressed: () async {
                  final r = await pickFolder(context, c, title: 'Move to');
                  if (r != null) await c.moveSession(s.id, r.folder);
                },
              ),
              IconButton(
                key: Key('delete-conv-${s.id}'),
                tooltip: 'Delete conversation',
                visualDensity: VisualDensity.compact,
                icon: const Icon(Icons.delete_outline, size: 18),
                onPressed: () => confirmDeleteConversation(context, c, (id: s.id, title: s.title)),
              ),
            ],
          ),
          onLongPress: () => confirmDeleteConversation(context, c, (id: s.id, title: s.title)),
          onTap: () {
            c.open(s.id);
            Navigator.of(context).pop();
          },
        ),
      );
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) => ListView(
        children: [
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton.icon(
              key: const Key('new-folder'),
              icon: const Icon(Icons.create_new_folder_outlined, size: 18),
              label: const Text('New folder'),
              onPressed: () => showFolderDialog(context, c),
            ),
          ),
          ..._rows(context, null, 0),
        ],
      ),
    );
  }
}

/// A chosen folder (`folder` null is the top level), so "nowhere" and "cancelled" differ.
class FolderChoice {
  const FolderChoice(this.folder);
  final String? folder;
}

/// Names a new folder (inside [parent]) or renames [rename].
Future<void> showFolderDialog(BuildContext context, BragiController c, {String? parent, FolderView? rename}) async {
  var text = rename?.name ?? '';
  await showDialog<void>(
    context: context,
    builder: (d) {
      String? error;
      return StatefulBuilder(
        builder: (d, setState) => AlertDialog(
          title: Text(rename == null ? 'New folder' : 'Rename folder'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              _FolderNameField(initial: text, onChanged: (v) => text = v),
              if (error != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(error!, key: const Key('folder-error'), style: TextStyle(color: BragiColors.error))),
            ],
          ),
          actions: [
            TextButton(key: const Key('folder-cancel'), onPressed: () => Navigator.of(d).pop(), child: const Text('Cancel')),
            FilledButton(
              key: const Key('folder-save'),
              onPressed: () async {
                if (rename == null) {
                  if (await c.createFolder(text, parent)) {
                    if (d.mounted) Navigator.of(d).pop();
                  } else {
                    setState(() => error = c.notice);
                  }
                } else {
                  await c.renameFolder(rename.id, text);
                  if (c.notice != null) {
                    setState(() => error = c.notice);
                  } else if (d.mounted) {
                    Navigator.of(d).pop();
                  }
                }
              },
              child: Text(rename == null ? 'Create' : 'Rename'),
            ),
          ],
        ),
      );
    },
  );
}

/// Owns the text controller so it is disposed with the field, after the dialog's exit animation.
class _FolderNameField extends StatefulWidget {
  const _FolderNameField({required this.initial, required this.onChanged});

  final String initial;
  final ValueChanged<String> onChanged;

  @override
  State<_FolderNameField> createState() => _FolderNameFieldState();
}

class _FolderNameFieldState extends State<_FolderNameField> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return TextField(
      key: const Key('folder-name'),
      controller: _controller,
      autofocus: true,
      onChanged: widget.onChanged,
      decoration: const InputDecoration(labelText: 'Name'),
    );
  }
}

/// Lists the folders (indented) and the top level to move something into.
Future<FolderChoice?> pickFolder(BuildContext context, BragiController c, {required String title, String? exclude}) {
  // Not into itself or below it.
  bool below(FolderView f) {
    for (String? at = f.id; at != null; at = c.folders.where((x) => x.id == at).firstOrNull?.parent) {
      if (at == exclude) return true;
    }
    return false;
  }

  int depthOf(FolderView f) {
    var n = 0;
    for (String? at = f.parent; at != null; at = c.folders.where((x) => x.id == at).firstOrNull?.parent) {
      n++;
    }
    return n;
  }

  return showDialog<FolderChoice>(
    context: context,
    builder: (d) => SimpleDialog(
      title: Text(title),
      children: [
        SimpleDialogOption(key: const Key('pick-top'), onPressed: () => Navigator.of(d).pop(const FolderChoice(null)), child: const Text('No folder (top level)')),
        for (final f in c.folders.where((f) => exclude == null || !below(f)))
          SimpleDialogOption(
            key: Key('pick-${f.id}'),
            onPressed: () => Navigator.of(d).pop(FolderChoice(f.id)),
            child: Padding(padding: EdgeInsets.only(left: depthOf(f) * 16.0), child: Text(f.name)),
          ),
      ],
    ),
  );
}

Future<void> confirmDeleteFolder(BuildContext context, BragiController c, FolderView f) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (d) => AlertDialog(
      title: Text('Delete the folder “${f.name}”?'),
      content: const Text('Its chats and folders move up one level. No chat is deleted.', key: Key('folder-delete-text')),
      actions: [
        TextButton(key: const Key('folder-delete-cancel'), onPressed: () => Navigator.of(d).pop(false), child: const Text('Cancel')),
        FilledButton(
          key: const Key('folder-delete-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: () => Navigator.of(d).pop(true),
          child: const Text('Delete folder'),
        ),
      ],
    ),
  );
  if (ok == true) await c.deleteFolder(f.id);
}
