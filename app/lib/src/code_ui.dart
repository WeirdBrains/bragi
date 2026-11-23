// Code mode's widgets: the Chat/Code toggle, the project bar and picker, the file browser for
// context, the review of proposed changes, the shell panel, and the bell's notification list.
import 'package:file_picker/file_picker.dart';
import 'package:material_ui/material_ui.dart';

import 'code_api.dart';
import 'code_controller.dart';
import 'controller.dart';
import 'desktop_pair_screen.dart';
import 'theme.dart';

/// Chat or Code, for what the message box sends; the conversation is the same.
class ModeToggle extends StatelessWidget {
  const ModeToggle({super.key, required this.code});

  final CodeController code;

  @override
  Widget build(BuildContext context) => SegmentedButton<bool>(
    key: const Key('mode-toggle'),
    showSelectedIcon: false,
    style: const ButtonStyle(visualDensity: VisualDensity.compact),
    segments: const [
      ButtonSegment(value: false, label: Text('Chat', key: Key('mode-chat'))),
      ButtonSegment(value: true, label: Text('Code', key: Key('mode-code'))),
    ],
    selected: {code.on},
    onSelectionChanged: (s) => code.setMode(s.first),
  );
}

/// The bell: every notice shown, newest first, with a dot for unread ones.
class NotificationBell extends StatelessWidget {
  const NotificationBell({super.key, required this.code});

  final CodeController code;

  @override
  Widget build(BuildContext context) {
    final n = code.unread;
    return IconButton(
      key: const Key('bell'),
      tooltip: 'Notifications',
      onPressed: () => _open(context),
      icon: Badge(isLabelVisible: n > 0, label: Text('$n'), child: const Icon(Icons.notifications_outlined)),
    );
  }

  void _open(BuildContext context) {
    final list = code.notifications;
    code.seen();
    showModalBottomSheet<void>(
      context: context,
      builder: (context) => SafeArea(
        child: list.isEmpty
            ? const Padding(
                padding: EdgeInsets.all(24),
                child: Text('No notifications yet.', key: Key('notifications-empty')),
              )
            : ListView(
                key: const Key('notifications'),
                shrinkWrap: true,
                children: [
                  for (final x in list)
                    ListTile(
                      dense: true,
                      leading: Icon(switch (x.level) {
                        'ok' => Icons.check_circle_outline,
                        'ask' => Icons.help_outline,
                        'error' => Icons.error_outline,
                        _ => Icons.info_outline,
                      }, color: x.level == 'error' ? BragiColors.error : BragiColors.textSecondary),
                      title: Text(x.text),
                      subtitle: Text(_when(x.atMs)),
                    ),
                ],
              ),
      ),
    );
  }

  static String _when(int ms) {
    final t = DateTime.fromMillisecondsSinceEpoch(ms);
    String two(int v) => v.toString().padLeft(2, '0');
    return '${t.year}-${two(t.month)}-${two(t.day)} ${two(t.hour)}:${two(t.minute)}';
  }
}

/// Code mode's bar over the message box: the project, its context, and the shell.
class CodeBar extends StatelessWidget {
  const CodeBar({super.key, required this.chat});

  final BragiController chat;

  @override
  Widget build(BuildContext context) {
    final c = chat.code;
    final p = c.project;
    final ctx = c.context;
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 6, 10, 0),
      child: Wrap(
        spacing: 6,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          ActionChip(
            key: const Key('project'),
            avatar: Icon(p?.desktopId == null ? Icons.folder_outlined : Icons.computer, size: 16),
            label: Text(p == null ? 'Open a project' : p.place, overflow: TextOverflow.ellipsis),
            onPressed: () => pickProject(context, chat),
          ),
          if (p != null)
            ActionChip(
              key: const Key('context'),
              avatar: const Icon(Icons.attach_file, size: 16),
              label: Text(
                ctx == null || ctx.files.isEmpty
                    ? 'Add files'
                    : '${ctx.files.length} file${ctx.files.length == 1 ? '' : 's'} · ${humanSize(ctx.totalBytes.toInt())}',
              ),
              onPressed: () => showModalBottomSheet<void>(
                context: context,
                isScrollControlled: true,
                builder: (_) => FractionallySizedBox(heightFactor: 0.8, child: ContextPicker(code: c)),
              ),
            ),
          if (p != null)
            ActionChip(
              key: const Key('shell'),
              avatar: const Icon(Icons.terminal, size: 16),
              label: const Text('Run'),
              onPressed: () => _askCommand(context, c),
            ),
          if (c.applied.isNotEmpty)
            ActionChip(key: const Key('undo-apply'), avatar: const Icon(Icons.undo, size: 16), label: const Text('Undo'), onPressed: c.undo),
        ],
      ),
    );
  }

  static Future<void> _askCommand(BuildContext context, CodeController c) async {
    final t = TextEditingController();
    final cmd = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Run in ${c.project?.name ?? 'the project'}'),
        content: TextField(
          key: const Key('shell-command'),
          controller: t,
          autofocus: true,
          autocorrect: false,
          style: const TextStyle(fontFamily: 'monospace'),
          decoration: InputDecoration(hintText: 'cargo test', helperText: c.project?.desktopId == null ? null : 'Runs on the desktop, as its user.'),
          onSubmitted: (v) => Navigator.pop(context, v),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
          FilledButton(key: const Key('shell-run'), onPressed: () => Navigator.pop(context, t.text), child: const Text('Run')),
        ],
      ),
    );
    t.dispose();
    if (cmd != null && cmd.trim().isNotEmpty) await c.runShell(cmd);
  }
}

/// Opens a project: a shared folder on a paired desktop, or (on a computer) a local folder.
Future<void> pickProject(BuildContext context, BragiController chat) => showModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  builder: (_) => FractionallySizedBox(heightFactor: 0.8, child: _ProjectPicker(chat: chat)),
);

class _ProjectPicker extends StatelessWidget {
  const _ProjectPicker({required this.chat});

  final BragiController chat;

  Future<void> _desktopRoot(BuildContext context, DesktopView d, int i) async {
    final c = chat.code;
    final t = TextEditingController();
    final sub = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(d.roots[i].name),
        content: TextField(
          key: const Key('project-sub'),
          controller: t,
          autocorrect: false,
          decoration: const InputDecoration(hintText: 'a folder in it (empty: all of it)', helperText: 'A new folder is made with starter files.'),
          onSubmitted: (v) => Navigator.pop(context, v),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
          FilledButton(key: const Key('project-open'), onPressed: () => Navigator.pop(context, t.text), child: const Text('Open')),
        ],
      ),
    );
    t.dispose();
    if (sub == null) return;
    if (await c.openDesktop(d.id, i, sub.trim()) && context.mounted) Navigator.pop(context);
  }

  Future<void> _local(BuildContext context) async {
    final path = await FilePicker.platform.getDirectoryPath(dialogTitle: 'Project folder');
    if (path == null) return;
    if (await chat.code.openLocal(path) && context.mounted) Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) {
    final c = chat.code;
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) {
        final desktops = c.desktops;
        return ListView(
          key: const Key('project-picker'),
          padding: const EdgeInsets.all(12),
          children: [
            const Text('Open a project', style: TextStyle(fontSize: 16, fontWeight: FontWeight.w600)),
            if (c.localAvailable)
              ListTile(
                key: const Key('project-local'),
                leading: const Icon(Icons.folder_open),
                title: const Text('A folder on this computer'),
                onTap: () => _local(context),
              ),
            for (final d in desktops) ...[
              ListTile(
                dense: true,
                leading: const Icon(Icons.computer),
                title: Text(d.label, style: const TextStyle(fontWeight: FontWeight.w600)),
                subtitle: d.roots.isEmpty ? const Text('Shares no folders yet: run `bragi linker roots add DIR` there.') : null,
              ),
              for (final (i, r) in d.roots.indexed)
                ListTile(
                  key: Key('root-${d.id}-$i'),
                  contentPadding: const EdgeInsets.only(left: 40, right: 16),
                  leading: const Icon(Icons.folder_outlined),
                  title: Text(r.name),
                  subtitle: Text(r.path, overflow: TextOverflow.ellipsis),
                  onTap: () => _desktopRoot(context, d, i),
                ),
            ],
            ListTile(
              key: const Key('pair-desktop-open'),
              leading: const Icon(Icons.qr_code_scanner),
              title: const Text('Pair with desktop'),
              subtitle: const Text('Work on a computer\'s projects from here'),
              onTap: () => Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => DesktopPairScreen(code: c))),
            ),
            if (c.project != null)
              ListTile(
                leading: const Icon(Icons.close),
                title: Text('Close ${c.project!.name}'),
                onTap: () {
                  c.closeProject();
                  Navigator.pop(context);
                },
              ),
          ],
        );
      },
    );
  }
}

/// Browse the project and pick files and folders for the code context.
class ContextPicker extends StatefulWidget {
  const ContextPicker({super.key, required this.code});

  final CodeController code;

  @override
  State<ContextPicker> createState() => _ContextPickerState();
}

class _ContextPickerState extends State<ContextPicker> {
  late final Set<String> _picked = {...?widget.code.context?.targets};
  late String _dir = widget.code.project?.base ?? '';
  Future<List<EntryView>>? _entries;

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _load() => _entries = widget.code.list(_dir);

  void _go(String dir) => setState(() {
    _dir = dir;
    _load();
  });

  String get _up {
    final i = _dir.lastIndexOf('/');
    return i < 0 ? '' : _dir.substring(0, i);
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        ListTile(
          title: Text(_dir.isEmpty ? '/' : _dir, style: const TextStyle(fontFamily: 'monospace')),
          leading: _dir.isEmpty
              ? const Icon(Icons.folder)
              : IconButton(key: const Key('dir-up'), icon: const Icon(Icons.arrow_upward), onPressed: () => _go(_up)),
          trailing: FilledButton(
            key: const Key('context-use'),
            onPressed: () async {
              final ok = await widget.code.setContext(_picked.toList()..sort());
              if (ok && context.mounted) Navigator.pop(context);
            },
            child: Text('Use ${_picked.length}'),
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: FutureBuilder<List<EntryView>>(
            future: _entries,
            builder: (context, snap) {
              if (snap.hasError) return Padding(padding: const EdgeInsets.all(16), child: Text('${snap.error}'));
              final list = snap.data;
              if (list == null) return const Center(child: CircularProgressIndicator());
              if (list.isEmpty) return const Center(child: Text('Empty folder.'));
              return ListView(
                key: const Key('entries'),
                children: [
                  for (final e in list)
                    CheckboxListTile(
                      key: Key('entry-${e.path}'),
                      dense: true,
                      controlAffinity: ListTileControlAffinity.leading,
                      value: _picked.contains(e.path),
                      onChanged: (v) => setState(() => v == true ? _picked.add(e.path) : _picked.remove(e.path)),
                      title: Text(e.name + (e.dir ? '/' : '')),
                      subtitle: e.dir ? null : Text(humanSize(e.size.toInt())),
                      secondary: e.dir ? IconButton(key: Key('open-${e.path}'), icon: const Icon(Icons.chevron_right), onPressed: () => _go(e.path)) : null,
                    ),
                ],
              );
            },
          ),
        ),
      ],
    );
  }
}

/// The changes an answer proposes: each file's hunks, each one kept or dropped, then Apply.
class ReviewCard extends StatelessWidget {
  const ReviewCard({super.key, required this.code});

  final CodeController code;

  @override
  Widget build(BuildContext context) {
    final r = code.review!;
    return Padding(
      key: const Key('review'),
      padding: const EdgeInsets.fromLTRB(10, 6, 10, 0),
      child: Material(
        color: BragiColors.surface,
        shape: RoundedRectangleBorder(
          side: BorderSide(color: BragiColors.accent),
          borderRadius: BorderRadius.circular(10),
        ),
        clipBehavior: Clip.antiAlias,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 320),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: ListView(
                  shrinkWrap: true,
                  padding: const EdgeInsets.all(8),
                  children: [
                    for (final (fi, f) in r.files.indexed)
                      ExpansionTile(
                        key: Key('review-file-$fi'),
                        initiallyExpanded: r.files.length <= 2,
                        tilePadding: EdgeInsets.zero,
                        title: Text(f.path, style: const TextStyle(fontFamily: 'monospace', fontSize: 13)),
                        subtitle: Text('${f.created ? 'new · ' : ''}+${f.added} −${f.removed}', style: const TextStyle(fontSize: 11)),
                        children: [
                          for (final (hi, h) in f.hunks.indexed)
                            Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [
                                CheckboxListTile(
                                  key: Key('hunk-$fi-$hi'),
                                  dense: true,
                                  contentPadding: EdgeInsets.zero,
                                  controlAffinity: ListTileControlAffinity.leading,
                                  value: h.accepted,
                                  onChanged: (v) => code.setHunk(fi, hi, v ?? false),
                                  title: Text(
                                    h.header,
                                    style: TextStyle(fontFamily: 'monospace', fontSize: 11, color: BragiColors.textSecondary),
                                  ),
                                ),
                                for (final l in h.lines)
                                  Container(
                                    color: switch (l.tag) {
                                      'added' => BragiColors.warm.withValues(alpha: 0.15),
                                      'removed' => BragiColors.error.withValues(alpha: 0.15),
                                      _ => null,
                                    },
                                    child: Text(
                                      '${switch (l.tag) {
                                        'added' => '+',
                                        'removed' => '-',
                                        _ => ' ',
                                      }} ${l.text}',
                                      style: TextStyle(fontFamily: 'monospace', fontSize: 12, color: h.accepted ? null : BragiColors.textSecondary),
                                    ),
                                  ),
                              ],
                            ),
                        ],
                      ),
                  ],
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(8, 0, 8, 8),
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    TextButton(key: const Key('review-discard'), onPressed: code.busy ? null : code.discard, child: const Text('Discard')),
                    const SizedBox(width: 8),
                    FilledButton(key: const Key('review-apply'), onPressed: code.busy ? null : code.apply, child: const Text('Apply')),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// A shell run's output as it streams, with Stop.
class ShellPanel extends StatelessWidget {
  const ShellPanel({super.key, required this.code});

  final CodeController code;

  @override
  Widget build(BuildContext context) {
    final s = code.shell!;
    return Padding(
      key: const Key('shell-panel'),
      padding: const EdgeInsets.fromLTRB(10, 6, 10, 0),
      child: Material(
        color: BragiColors.surface,
        shape: RoundedRectangleBorder(
          side: BorderSide(color: BragiColors.border),
          borderRadius: BorderRadius.circular(10),
        ),
        child: Container(
          padding: const EdgeInsets.all(8),
          constraints: const BoxConstraints(maxHeight: 240),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      '\$ ${s.command}',
                      style: const TextStyle(fontFamily: 'monospace', fontWeight: FontWeight.w600),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  Text(
                    s.status,
                    key: const Key('shell-status'),
                    style: TextStyle(fontSize: 11, color: BragiColors.textSecondary),
                  ),
                  if (!s.done) IconButton(key: const Key('shell-stop'), tooltip: 'Stop', onPressed: code.stopShell, icon: const Icon(Icons.stop, size: 18)),
                  if (s.done) IconButton(key: const Key('shell-close'), tooltip: 'Close', onPressed: code.closeShell, icon: const Icon(Icons.close, size: 18)),
                ],
              ),
              Flexible(
                child: SingleChildScrollView(
                  reverse: true,
                  child: SelectableText(
                    s.output.toString(),
                    key: const Key('shell-output'),
                    style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
