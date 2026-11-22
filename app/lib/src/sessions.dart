// The Sessions section of the drawer: sessions in nested folders, each sending to the Weirdnet
// pool or one endpoint. Tap a session to open it, tap a folder to fold it, drag a session (or a
// folder) onto a folder or the top level, and use a row's menu to rename, move or delete it.
// Every change is one core call; the tree comes back in the state, drawn as the core orders it.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'theme.dart';

/// What is being dragged: a session or a folder.
class Dragged {
  const Dragged(this.kind, this.id);

  final String kind;
  final String id;
}

Future<String?> askName(BuildContext context, String title, {String initial = '', String hint = 'Name'}) {
  final text = TextEditingController(text: initial);
  return showDialog<String>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: Text(title),
      content: TextField(
        key: const Key('name-field'),
        controller: text,
        autofocus: true,
        decoration: InputDecoration(hintText: hint),
        onSubmitted: (v) => Navigator.of(ctx).pop(v),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.of(ctx).pop(), child: const Text('Cancel')),
        FilledButton(key: const Key('name-ok'), onPressed: () => Navigator.of(ctx).pop(text.text), child: const Text('OK')),
      ],
    ),
  );
}

/// The folders as (depth, folder), depth first, without `skip` and what is inside it.
List<(int, FolderView)> folderPaths(List<FolderView> folders, {String? skip}) {
  final out = <(int, FolderView)>[];
  void walk(String? parent, int depth) {
    final here = folders.where((f) => f.parent == parent && f.id != skip).toList()
      ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    for (final f in here) {
      out.add((depth, f));
      if (depth < 64) walk(f.id, depth + 1);
    }
  }

  walk(null, 0);
  return out;
}

/// Picks where to move something: the top level or a folder. Returns `''` for the top level.
Future<String?> askFolder(BuildContext context, BragiController c, {String? skip}) {
  return showDialog<String>(
    context: context,
    builder: (ctx) => SimpleDialog(
      title: const Text('Move to'),
      children: [
        SimpleDialogOption(key: const Key('move-top'), onPressed: () => Navigator.of(ctx).pop(''), child: const Text('Top level')),
        for (final (d, f) in folderPaths(c.folders, skip: skip))
          SimpleDialogOption(
            key: Key('move-to-${f.id}'),
            onPressed: () => Navigator.of(ctx).pop(f.id),
            child: Padding(padding: EdgeInsets.only(left: 16.0 * d), child: Text('📁 ${f.name}')),
          ),
      ],
    ),
  );
}

/// A new session: its name, what it sends to, and (optionally) its model.
Future<void> newSession(BuildContext context, BragiController c, {String? folder}) async {
  final name = TextEditingController();
  String? endpoint;
  String? model;
  final ok = await showDialog<bool>(
    context: context,
    builder: (ctx) => StatefulBuilder(
      builder: (ctx, set) {
        final models = endpoint == null ? c.state?.models.map((m) => m.id).toList() ?? const <String>[] : c.endpoint(endpoint!)?.models ?? const <String>[];
        return AlertDialog(
          title: const Text('New session'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(key: const Key('session-name'), controller: name, autofocus: true, decoration: const InputDecoration(labelText: 'Name')),
              const SizedBox(height: 12),
              DropdownButtonFormField<String?>(
                key: const Key('session-target'),
                initialValue: endpoint,
                decoration: const InputDecoration(labelText: 'Sends to'),
                items: [
                  const DropdownMenuItem<String?>(value: null, child: Text('Weirdnet')),
                  for (final e in c.endpoints) DropdownMenuItem<String?>(value: e.id, child: Text(e.name)),
                ],
                onChanged: (v) => set(() {
                  endpoint = v;
                  model = null;
                }),
              ),
              const SizedBox(height: 12),
              DropdownButtonFormField<String?>(
                key: const Key('session-model'),
                initialValue: model,
                decoration: const InputDecoration(labelText: 'Model'),
                items: [
                  const DropdownMenuItem<String?>(value: null, child: Text('The first one it lists')),
                  for (final m in models) DropdownMenuItem<String?>(value: m, child: Text(m)),
                ],
                onChanged: (v) => set(() => model = v),
              ),
            ],
          ),
          actions: [
            TextButton(onPressed: () => Navigator.of(ctx).pop(false), child: const Text('Cancel')),
            FilledButton(key: const Key('session-ok'), onPressed: () => Navigator.of(ctx).pop(true), child: const Text('Create')),
          ],
        );
      },
    ),
  );
  if (ok == true) await c.createSession(name.text, endpointId: endpoint, model: model, folder: folder);
}

Future<void> confirmDeleteFolder(BuildContext context, BragiController c, FolderView f) async {
  final yes = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: Text('Delete the folder ${f.name}?'),
      content: const Text('Its sessions and folders move up to the folder it is in. No conversation is deleted.'),
      actions: [
        TextButton(onPressed: () => Navigator.of(ctx).pop(false), child: const Text('Cancel')),
        FilledButton(key: const Key('delete-folder-ok'), onPressed: () => Navigator.of(ctx).pop(true), child: const Text('Delete folder')),
      ],
    ),
  );
  if (yes == true) await c.deleteFolder(f.id);
}

Future<void> confirmDeleteSession(BuildContext context, BragiController c, SessionView s) async {
  final yes = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: Text('Delete the session ${s.name}?'),
      content: const Text('Its conversations are kept: they move to the Weirdnet session.'),
      actions: [
        TextButton(onPressed: () => Navigator.of(ctx).pop(false), child: const Text('Cancel')),
        FilledButton(key: const Key('delete-session-ok'), onPressed: () => Navigator.of(ctx).pop(true), child: const Text('Delete session')),
      ],
    ),
  );
  if (yes == true) await c.deleteSession(s.id);
}

Future<void> _drop(BragiController c, Dragged d, String? folder) async {
  if (d.kind == 'session') {
    await c.moveSession(d.id, folder);
  } else if (d.id != folder) {
    await c.moveFolder(d.id, folder);
  }
}

class SessionsSection extends StatelessWidget {
  const SessionsSection({super.key, required this.controller, required this.onOpened});

  final BragiController controller;

  /// Called after a session opens (the drawer closes).
  final VoidCallback onOpened;

  Widget _row(TreeRowView row) {
    final c = controller;
    if (row.kind == 'folder') {
      final f = c.folder(row.id);
      return f == null ? const SizedBox.shrink() : _FolderRow(controller: c, folder: f, depth: row.depth);
    }
    final s = c.session(row.id);
    return s == null ? const SizedBox.shrink() : _SessionRow(controller: c, session: s, depth: row.depth, onOpened: onOpened);
  }

  @override
  Widget build(BuildContext context) {
    final c = controller;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // The header is also the drop target for the top level.
        DragTarget<Dragged>(
          onAcceptWithDetails: (d) => _drop(c, d.data, null),
          builder: (_, over, _) => Container(
            key: const Key('sessions-top'),
            color: over.isNotEmpty ? BragiColors.selection : null,
            padding: const EdgeInsets.fromLTRB(16, 4, 4, 0),
            child: Row(
              children: [
                const Expanded(child: Text('SESSIONS', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary))),
                IconButton(
                  key: const Key('new-folder'),
                  tooltip: 'New folder',
                  visualDensity: VisualDensity.compact,
                  icon: const Icon(Icons.create_new_folder_outlined, size: 18),
                  onPressed: () async {
                    final n = await askName(context, 'New folder');
                    if (n != null) await c.createFolder(n);
                  },
                ),
                IconButton(
                  key: const Key('new-session'),
                  tooltip: 'New session',
                  visualDensity: VisualDensity.compact,
                  icon: const Icon(Icons.add, size: 18),
                  onPressed: () => newSession(context, c),
                ),
              ],
            ),
          ),
        ),
        for (final row in c.tree) _row(row),
        if (c.route case final r?)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 6),
            child: Text(r.detail, key: const Key('drawer-route'), style: const TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
          ),
      ],
    );
  }
}

class _FolderRow extends StatelessWidget {
  const _FolderRow({required this.controller, required this.folder, required this.depth});

  final BragiController controller;
  final FolderView folder;
  final int depth;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final f = folder;
    final row = DragTarget<Dragged>(
      onWillAcceptWithDetails: (d) => d.data.id != f.id,
      onAcceptWithDetails: (d) => _drop(c, d.data, f.id),
      builder: (_, over, _) => Container(
        color: over.isNotEmpty ? BragiColors.selection : null,
        child: ListTile(
          key: Key('folder-${f.id}'),
          dense: true,
          contentPadding: EdgeInsets.only(left: 8.0 + 16.0 * depth, right: 0),
          leading: Icon(f.collapsed ? Icons.chevron_right : Icons.expand_more, size: 18),
          minLeadingWidth: 18,
          title: Text('📁 ${f.name}', overflow: TextOverflow.ellipsis),
          onTap: () => c.toggleFolder(f.id),
          trailing: PopupMenuButton<String>(
            key: Key('folder-menu-${f.id}'),
            onSelected: (v) async {
              switch (v) {
                case 'session':
                  await newSession(context, c, folder: f.id);
                case 'folder':
                  final n = await askName(context, 'New folder in ${f.name}');
                  if (n != null) await c.createFolder(n, parent: f.id);
                case 'rename':
                  final n = await askName(context, 'Rename folder', initial: f.name);
                  if (n != null) await c.renameFolder(f.id, n);
                case 'move':
                  if (!context.mounted) return;
                  final to = await askFolder(context, c, skip: f.id);
                  if (to != null) await c.moveFolder(f.id, to.isEmpty ? null : to);
                case 'delete':
                  if (context.mounted) await confirmDeleteFolder(context, c, f);
              }
            },
            itemBuilder: (_) => const [
              PopupMenuItem(value: 'session', child: Text('New session here')),
              PopupMenuItem(value: 'folder', child: Text('New folder here')),
              PopupMenuItem(value: 'rename', child: Text('Rename')),
              PopupMenuItem(value: 'move', child: Text('Move to…')),
              PopupMenuItem(value: 'delete', child: Text('Delete folder')),
            ],
          ),
        ),
      ),
    );
    return LongPressDraggable<Dragged>(
      data: Dragged('folder', f.id),
      feedback: Material(color: Colors.transparent, child: Chip(label: Text('📁 ${f.name}'))),
      child: row,
    );
  }
}

class _SessionRow extends StatelessWidget {
  const _SessionRow({required this.controller, required this.session, required this.depth, required this.onOpened});

  final BragiController controller;
  final SessionView session;
  final int depth;
  final VoidCallback onOpened;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final s = session;
    final weirdnet = s.endpointId == null;
    final tile = ListTile(
      key: Key('session-${s.id}'),
      dense: true,
      selected: c.activeSessionId == s.id,
      contentPadding: EdgeInsets.only(left: 26.0 + 16.0 * depth, right: 0),
      leading: Icon(weirdnet ? Icons.hub_outlined : Icons.link, size: 18),
      minLeadingWidth: 18,
      title: Text(s.name, overflow: TextOverflow.ellipsis),
      subtitle: Text('${s.targetName}${s.model != null ? ' · ${s.model}' : ''}', style: const TextStyle(fontSize: 11), overflow: TextOverflow.ellipsis),
      onTap: () async {
        await c.switchSession(s.id);
        onOpened();
      },
      trailing: PopupMenuButton<String>(
        key: Key('session-menu-${s.id}'),
        onSelected: (v) async {
          switch (v) {
            case 'rename':
              final n = await askName(context, 'Rename session', initial: s.name);
              if (n != null) await c.renameSession(s.id, n);
            case 'move':
              if (!context.mounted) return;
              final to = await askFolder(context, c);
              if (to != null) await c.moveSession(s.id, to.isEmpty ? null : to);
            case 'delete':
              if (context.mounted) await confirmDeleteSession(context, c, s);
            default:
              if (v.startsWith('target:')) await c.setSessionEndpoint(s.id, v == 'target:' ? null : v.substring(7));
          }
        },
        itemBuilder: (_) => [
          const PopupMenuItem(value: 'rename', child: Text('Rename')),
          const PopupMenuItem(value: 'move', child: Text('Move to…')),
          if (s.id != 'weirdnet') ...[
            const PopupMenuItem(value: 'target:', child: Text('Send to Weirdnet')),
            for (final e in c.endpoints) PopupMenuItem(value: 'target:${e.id}', child: Text('Send to ${e.name}')),
            const PopupMenuItem(value: 'delete', child: Text('Delete session')),
          ],
        ],
      ),
    );
    return LongPressDraggable<Dragged>(
      data: Dragged('session', s.id),
      feedback: Material(color: Colors.transparent, child: Chip(label: Text(s.name))),
      child: tile,
    );
  }
}
