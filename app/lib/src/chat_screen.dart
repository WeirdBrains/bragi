// The chat: pick a model from the catalog, ask, watch it stream, stop it, continue it. In code
// mode the same box asks for changes to a project (code_ui.dart), in the same conversation.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'code_ui.dart';
import 'controller.dart';
import 'markdown.dart';
import 'delete_ui.dart';
import 'sessions_ui.dart';
import 'models_ui.dart';
import 'settings_ui.dart';
import 'theme_ui.dart';
import 'theme.dart';
import 'training_ui.dart';
import 'video.dart';
import 'rust/api/bragi.dart' as rust;

const _scripted = bool.fromEnvironment('BRAGI_DEMO');

class ChatScreen extends StatefulWidget {
  const ChatScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final _prompt = TextEditingController();
  final _link = TextEditingController();
  final _scroll = ScrollController();

  /// Follow the newest text; false once the reader scrolls up.
  bool _stick = true;

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_onScroll);
    widget.controller.code.addListener(_codeChanged);
  }

  void _codeChanged() {
    if (mounted) setState(() {});
  }

  /// Up and Down on a hardware keyboard walk the prompts sent before (the chat's, or the open
  /// project's), as in the terminal app; only while the box holds one line.
  KeyEventResult _historyKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent || _prompt.text.contains('\n')) {
      return KeyEventResult.ignored;
    }
    final code = widget.controller.code;
    final String? next;
    if (e.logicalKey == LogicalKeyboardKey.arrowUp) {
      next = code.historyUp(_prompt.text);
    } else if (e.logicalKey == LogicalKeyboardKey.arrowDown) {
      next = code.historyDown();
    } else {
      return KeyEventResult.ignored;
    }
    if (next == null) return KeyEventResult.handled;
    _prompt.value = TextEditingValue(
      text: next,
      selection: TextSelection.collapsed(offset: next.length),
    );
    return KeyEventResult.handled;
  }

  void _onScroll() {
    if (!_scroll.hasClients) return;
    final atBottom =
        _scroll.position.maxScrollExtent - _scroll.position.pixels <= 24;
    if (atBottom != _stick) setState(() => _stick = atBottom);
  }

  void _jumpToLatest() {
    setState(() => _stick = true);
    if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
  }

  @override
  void dispose() {
    widget.controller.code.removeListener(_codeChanged);
    _prompt.dispose();
    _link.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// The system picker; the scripted demo (no one to tap a system dialog) attaches a sample file.
  Future<void> _attach() async {
    final c = widget.controller;
    if (_scripted) {
      c.attachFile(
        'sample-notes.md',
        utf8.encode('# Notes\n- buy milk\n- write the B5 milestone\n'),
      );
      return;
    }
    final r = await FilePicker.platform.pickFiles(withData: true);
    for (final f in r?.files ?? const <PlatformFile>[]) {
      final bytes = f.bytes;
      if (bytes != null) c.attachFile(f.name, bytes);
    }
  }

  /// A clip from the gallery or files (the scripted demo writes a sample clip and its scripted
  /// decoder reads it); the core samples it for the current model.
  Future<void> _attachVideo() async {
    final c = widget.controller;
    if (_scripted) {
      final f = File('${Directory.systemTemp.path}/bragi-sample-clip.mp4');
      await f.writeAsBytes(sampleJpeg, flush: true);
      await c.attachVideo(f.path, name: 'sample-clip.mp4');
      return;
    }
    final r = await FilePicker.platform.pickFiles(type: FileType.video);
    final f = r?.files.firstOrNull;
    if (f?.path != null) await c.attachVideo(f!.path!, name: f.name);
  }

  /// This device's video limits for the current model: what is reported, and your own
  /// `key=value` values that fill gaps and only lower what is reported.
  Future<void> _videoLimits() async {
    final c = widget.controller;
    final now = c.videoLimits ?? await c.refreshVideoLimits();
    if (!mounted || now == null) return;
    final field = TextEditingController(text: now.user);
    final text = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('Video limits for ${now.model}'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Reported: ${now.reported.isEmpty ? 'nothing' : now.reported}',
              style: const TextStyle(fontSize: 12),
            ),
            const SizedBox(height: 8),
            TextField(
              key: const Key('video-limits-field'),
              controller: field,
              decoration: InputDecoration(
                labelText: 'Yours (key=value …)',
                hintText: 'image_tokens=258 max_images=32',
                helperText:
                    'Fills what isn\'t reported, only lowers what is. Keys: ${now.keys.join(', ')}',
                helperMaxLines: 4,
              ),
            ),
          ],
        ),
        actions: [
          TextButton(
            key: const Key('video-limits-clear'),
            onPressed: () => Navigator.pop(ctx, ''),
            child: const Text('Clear'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Cancel'),
          ),
          FilledButton(
            key: const Key('video-limits-save'),
            onPressed: () => Navigator.pop(ctx, field.text),
            child: const Text('Save'),
          ),
        ],
      ),
    );
    field.dispose();
    if (text != null) await c.setVideoLimits(text);
  }

  void _send() {
    final text = _prompt.text;
    final c = widget.controller;
    if (c.code.on && c.code.project == null) {
      // Kept in the box until there is a project to ask about.
      c.say('Open a project first (the folder button).');
      return;
    }
    _prompt.clear();
    c.code.historyPush(text);
    if (c.code.on) {
      c.code.ask(text);
    } else {
      c.send(text);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.controller;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_stick && _scroll.hasClients) {
        _scroll.jumpTo(_scroll.position.maxScrollExtent);
      }
    });
    final wide = MediaQuery.sizeOf(context).width >= 1024;
    final showSidebar = wide && !c.sidebarCollapsed;
    rust.PoolView? pool;
    for (final p in c.pools) {
      if (p.active) {
        pool = p;
        break;
      }
    }
    return Scaffold(
      appBar: AppBar(
        leading: wide
            ? IconButton(
                key: const Key('toggle-sidebar'),
                tooltip: c.sidebarCollapsed ? 'Show history' : 'Hide history',
                onPressed: c.toggleSidebar,
                icon: Icon(c.sidebarCollapsed ? Icons.menu : Icons.menu_open),
              )
            : null,
        title: Text(
          [
            c.current?.title ?? 'Bragi',
            if (pool != null) pool.name,
          ].join(' · '),
          overflow: TextOverflow.ellipsis,
        ),
        actions: [
          ModeToggle(code: c.code),
          TrainingToggle(controller: c),
          NotificationBell(code: c.code),
          IconButton(
            key: const Key('new-chat'),
            tooltip: 'New chat',
            onPressed: c.newChat,
            icon: const Icon(Icons.add_comment_outlined),
          ),
        ],
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(92),
          child: Column(
            children: [
              TargetBar(controller: c),
              _Models(controller: c),
            ],
          ),
        ),
      ),
      drawer: showSidebar ? null : _History(controller: c, link: _link),
      body: SafeArea(
        child: Row(
          children: [
            if (showSidebar) ...[
              SizedBox(
                width: 280,
                child: _History(controller: c, link: _link, persistent: true),
              ),
              VerticalDivider(width: 1, color: BragiColors.border),
            ],
            Expanded(
              child: Column(
                children: [
                  Expanded(
                    child: c.messages.isEmpty
                        ? Center(
                            child: Text(
                              'Ask your own cloud something.',
                              style: TextStyle(
                                color: BragiColors.textSecondary,
                              ),
                            ),
                          )
                        // Replies stay selectable; each reply's copy button gives its raw Markdown.
                        : Stack(
                            children: [
                              SelectionArea(
                                child: ListView.builder(
                                  key: const Key('messages'),
                                  controller: _scroll,
                                  padding: const EdgeInsets.all(12),
                                  itemCount: c.messages.length,
                                  itemBuilder: (context, i) => GestureDetector(
                                    key: Key('bubble-$i'),
                                    // Long-press (or right-click) a message for its menu; not while an answer streams.
                                    onLongPress: c.streaming
                                        ? null
                                        : () => messageMenu(
                                            context,
                                            c,
                                            c.messages[i],
                                          ),
                                    onSecondaryTap: c.streaming
                                        ? null
                                        : () => messageMenu(
                                            context,
                                            c,
                                            c.messages[i],
                                          ),
                                    child: _Bubble(
                                      key: ValueKey(
                                        'bubble-${c.current?.id}-$i',
                                      ),
                                      message: c.messages[i],
                                      streaming:
                                          c.streaming &&
                                          i == c.messages.length - 1,
                                      controller: c,
                                    ),
                                  ),
                                ),
                              ),
                              if (!_stick)
                                Positioned(
                                  bottom: 8,
                                  left: 0,
                                  right: 0,
                                  child: Center(
                                    child: FilledButton.icon(
                                      key: const Key('jump-latest'),
                                      onPressed: _jumpToLatest,
                                      icon: const Icon(
                                        Icons.arrow_downward,
                                        size: 16,
                                      ),
                                      label: const Text('Jump to latest'),
                                    ),
                                  ),
                                ),
                            ],
                          ),
                  ),
                  if (c.notice != null)
                    Padding(
                      padding: const EdgeInsets.all(6),
                      child: Text(
                        c.notice!,
                        style: TextStyle(
                          color: c.notice!.startsWith('Saved ')
                              ? BragiColors.warm
                              : BragiColors.error,
                          fontSize: 12,
                        ),
                      ),
                    ),
                  if (c.nodeSearchLine != null)
                    Padding(
                      padding: const EdgeInsets.all(6),
                      child: Text(
                        c.nodeSearchLine!,
                        key: const Key('node-search'),
                        style: TextStyle(
                          fontSize: 12,
                          color: switch (c.nodeSearch) {
                            'found' => BragiColors.warm,
                            'not_found' => BragiColors.busy,
                            _ => BragiColors.textSecondary,
                          },
                        ),
                      ),
                    ),
                  if (c.card != null) _Card(controller: c),
                  if (c.code.on && (c.code.review?.files.isNotEmpty ?? false))
                    ReviewCard(code: c.code),
                  if (c.code.on && c.code.shell != null)
                    ShellPanel(code: c.code),
                  if (c.code.on) CodeBar(chat: c),
                  if (c.videoReading != null ||
                      c.videoLimits != null ||
                      c.attachments.any((a) => a.kind == 'video'))
                    Padding(
                      padding: const EdgeInsets.fromLTRB(10, 4, 10, 0),
                      child: Row(
                        children: [
                          if (c.videoReading != null) ...[
                            const SizedBox(
                              width: 14,
                              height: 14,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            ),
                            const SizedBox(width: 6),
                            Expanded(
                              child: Text(
                                'Reading frames of ${c.videoReading}…',
                                key: const Key('video-reading'),
                                overflow: TextOverflow.ellipsis,
                                style: const TextStyle(fontSize: 12),
                              ),
                            ),
                          ] else
                            Flexible(
                              child: ActionChip(
                                key: const Key('video-limits'),
                                avatar: Icon(
                                  (c.videoLimits?.ready ?? true)
                                      ? Icons.movie_outlined
                                      : Icons.warning_amber,
                                  size: 16,
                                ),
                                label: Text(
                                  c.videoLimits?.chip ?? 'Video limits…',
                                  overflow: TextOverflow.ellipsis,
                                ),
                                onPressed: _videoLimits,
                              ),
                            ),
                        ],
                      ),
                    ),
                  if (c.attachments.isNotEmpty)
                    Padding(
                      padding: const EdgeInsets.fromLTRB(10, 4, 10, 0),
                      child: Wrap(
                        spacing: 6,
                        children: [
                          for (final (i, a) in c.attachments.indexed)
                            InputChip(
                              key: Key('chip-$i'),
                              label: Text(_attachmentLabel(a)),
                              avatar: a.kind == 'text'
                                  ? null
                                  : Icon(switch (a.kind) {
                                      'image' => Icons.image_outlined,
                                      'video' => Icons.movie_outlined,
                                      _ => Icons.graphic_eq,
                                    }, size: 16),
                              onDeleted: () => c.detach(i),
                            ),
                        ],
                      ),
                    ),
                  Container(
                    decoration: BoxDecoration(
                      border: Border(
                        top: BorderSide(color: BragiColors.border),
                      ),
                    ),
                    padding: const EdgeInsets.fromLTRB(4, 8, 10, 8),
                    child: Row(
                      children: [
                        if (!c.code.on) ...[
                          IconButton(
                            key: const Key('attach'),
                            visualDensity: VisualDensity.compact,
                            padding: EdgeInsets.zero,
                            constraints: const BoxConstraints(
                              minWidth: 32,
                              minHeight: 40,
                            ),
                            tooltip: 'Attach a file, image or audio',
                            onPressed: _attach,
                            icon: const Icon(Icons.attach_file),
                          ),
                          IconButton(
                            key: const Key('attach-video'),
                            visualDensity: VisualDensity.compact,
                            padding: EdgeInsets.zero,
                            constraints: const BoxConstraints(
                              minWidth: 32,
                              minHeight: 40,
                            ),
                            tooltip: 'Attach a video',
                            onPressed: c.videoReading == null
                                ? _attachVideo
                                : null,
                            icon: const Icon(Icons.videocam_outlined),
                          ),
                          IconButton(
                            key: const Key('agent'),
                            visualDensity: VisualDensity.compact,
                            padding: EdgeInsets.zero,
                            constraints: const BoxConstraints(
                              minWidth: 32,
                              minHeight: 40,
                            ),
                            tooltip: c.agent
                                ? 'Agent mode: on'
                                : 'Agent mode: off',
                            isSelected: c.agent,
                            style: c.agent
                                ? IconButton.styleFrom(
                                    backgroundColor: BragiColors.accent,
                                    foregroundColor: BragiColors.background,
                                  )
                                : null,
                            onPressed: c.toggleAgent,
                            icon: const Icon(Icons.smart_toy_outlined),
                          ),
                        ],
                        const SizedBox(width: 4),
                        Expanded(
                          child: Focus(
                            onKeyEvent: _historyKey,
                            child: TextField(
                              key: const Key('prompt'),
                              controller: _prompt,
                              minLines: 1,
                              maxLines: 4,
                              textInputAction: TextInputAction.send,
                              // The scripted demo types through `adb input`; the keyboard's autocorrect turned
                              // "What" into "Chat". Real use keeps the keyboard's own settings.
                              autocorrect: !_scripted,
                              enableSuggestions: !_scripted,
                              onSubmitted: (_) => _send(),
                              decoration: InputDecoration(
                                hintText: c.code.on
                                    ? 'Ask for a change…'
                                    : 'Ask anything…',
                                isDense: true,
                                border: const OutlineInputBorder(),
                              ),
                            ),
                          ),
                        ),
                        if (c.canContinue) ...[
                          const SizedBox(width: 6),
                          OutlinedButton(
                            key: const Key('continue'),
                            style: OutlinedButton.styleFrom(
                              visualDensity: VisualDensity.compact,
                              padding: const EdgeInsets.symmetric(
                                horizontal: 10,
                              ),
                            ),
                            onPressed: c.continueAnswer,
                            child: const Text('Continue'),
                          ),
                        ],
                        const SizedBox(width: 6),
                        c.streaming
                            ? OutlinedButton.icon(
                                key: const Key('stop'),
                                onPressed: c.stop,
                                icon: const Icon(Icons.stop, size: 16),
                                label: const Text('Stop'),
                              )
                            : FilledButton(
                                key: const Key('send'),
                                onPressed: _send,
                                child: const Text('Send'),
                              ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Card extends StatelessWidget {
  const _Card({required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final card = c.card!;
    return Container(
      key: const Key('approval'),
      margin: const EdgeInsets.fromLTRB(10, 6, 10, 0),
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        border: Border.all(color: BragiColors.accent),
        borderRadius: BorderRadius.circular(10),
        color: BragiColors.surface,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(card.summary, style: const TextStyle(fontSize: 14)),
          Text(
            'the node asks to ${card.tier} · only you can say yes',
            style: TextStyle(fontSize: 11, color: BragiColors.textSecondary),
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 8,
            children: [
              FilledButton(
                key: const Key('allow-once'),
                onPressed: () => c.answer('once'),
                child: const Text('Allow once'),
              ),
              OutlinedButton(
                key: const Key('allow-task'),
                onPressed: () => c.answer('task'),
                child: const Text('This task'),
              ),
              OutlinedButton(
                key: const Key('deny'),
                onPressed: () => c.answer('deny'),
                child: const Text('Deny'),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _Models extends StatelessWidget {
  const _Models({required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    if (c.models.isEmpty) {
      return Padding(
        padding: EdgeInsets.all(12),
        child: Text(
          'Looking for your inference nodes…',
          style: TextStyle(color: BragiColors.textSecondary),
        ),
      );
    }
    return SizedBox(
      height: 48,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        child: Row(
          children: [
            for (final m in c.models)
              Padding(
                padding: const EdgeInsets.only(right: 8),
                child: ChoiceChip(
                  key: Key('model-${m.id}'),
                  selected: c.model == m.id,
                  onSelected: (_) => c.pickModel(m.id),
                  avatar: CircleAvatar(
                    radius: 4,
                    backgroundColor: _dot(m.status),
                  ),
                  label: Text(
                    '${m.id} · ${_label(m)}${c.state!.nodeCount > 1 ? ' · ${m.nodes}/${c.state!.nodeCount} nodes' : ''}',
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }

  static Color _dot(String status) => switch (status) {
    'warm' => BragiColors.warm,
    'loading' => BragiColors.accent,
    'busy' => BragiColors.busy,
    _ => BragiColors.textSecondary,
  };

  static String _label(ModelView m) {
    final s = switch (m.status) {
      'loadable' => 'loads on use',
      final s => s,
    };
    final speed = m.tokensPerSec == null
        ? ''
        : ' · ${m.tokensPerSec!.round()} tok/s';
    return '$s$speed';
  }
}

class _Bubble extends StatelessWidget {
  const _Bubble({
    super.key,
    required this.message,
    required this.streaming,
    required this.controller,
  });

  final MessageView message;
  final bool streaming;
  final BragiController controller;

  /// Parses a reply's Markdown (bragi-core, through the controller's cache).

  @override
  Widget build(BuildContext context) {
    final mine = message.role == 'user';
    final model = message.model ?? '';
    final meta = mine
        ? null
        : switch (streaming ? 'streaming' : message.state) {
            'streaming' => '$model · answering…',
            'stopped' => '$model · stopped',
            'failed' => '$model · cut off',
            _ => model,
          };
    // What the pool did: served from the cache, answered by a stand-in, moved between nodes.
    final flags = [
      if (message.cacheHit) 'cached',
      if (message.standin != null) 'stand-in ${message.standin}',
      if (message.resumes == 1)
        'moved to another node'
      else if (message.resumes > 1)
        'moved nodes ${message.resumes}×',
      if (message.agent)
        message.tools.isEmpty
            ? 'agent task'
            : 'agent task · used ${message.tools.join(', ')}',
      // A turn made in code mode (the terminal app's), in the same conversation.
      if (message.mode == 'code') 'code mode',
    ];
    final line = mine
        ? null
        : [if (meta != null && meta.isNotEmpty) meta, ...flags].join(' · ');
    return Align(
      alignment: mine ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        constraints: const BoxConstraints(maxWidth: 560),
        margin: const EdgeInsets.symmetric(vertical: 5),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(
          color: mine ? BragiColors.selection : BragiColors.surface,
          borderRadius: BorderRadius.circular(12),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Saved messages show their files as cards (below); one still being sent, its labels.
            if (message.files.isEmpty)
              for (final a in message.attachments)
                Padding(
                  padding: const EdgeInsets.only(bottom: 4),
                  child: Text(
                    'attached: ${_attachmentLabel(a)}',
                    key: const Key('bubble-attachment'),
                    style: TextStyle(
                      fontSize: 12,
                      color: BragiColors.textSecondary,
                    ),
                  ),
                ),
            if (line != null && line.isNotEmpty)
              Text(
                line,
                key: const Key('bubble-meta'),
                style: TextStyle(
                  fontSize: 11,
                  color: BragiColors.textSecondary,
                ),
              ),
            if (!mine && message.reasoning.trim().isNotEmpty)
              _Thinking(
                message: message,
                streaming: streaming,
                controller: controller,
              ),
            if (mine || message.text.isEmpty)
              Text(
                message.text.isEmpty
                    // Nothing from the model yet: say so, until reasoning or text arrives.
                    ? (streaming && message.reasoning.trim().isEmpty
                          ? 'Waiting for a reply…'
                          : '…')
                    : message.text,
                key: message.text.isEmpty && streaming
                    ? const Key('bubble-waiting')
                    : null,
                style: const TextStyle(fontSize: 15, height: 1.4),
              )
            else
              MarkdownView(
                key: const Key('bubble-markdown'),
                doc: controller.markdown(
                  message.text,
                  streaming || message.state == 'streaming',
                ),
              ),
            if (!streaming)
              for (final f in message.files)
                if (controller.videoInfo(f.id) case final v?)
                  _VideoCard(controller: controller, file: f.id, info: v)
                else
                  _FileCard(controller: controller, file: f),
            if (message.reactions.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 6),
                child: Wrap(
                  spacing: 4,
                  runSpacing: 4,
                  children: [
                    for (final e in message.reactions)
                      InkWell(
                        key: Key('pill-$e'),
                        borderRadius: BorderRadius.circular(10),
                        onTap: streaming
                            ? null
                            : () => controller.react(message, e),
                        child: Container(
                          padding: const EdgeInsets.symmetric(
                            horizontal: 7,
                            vertical: 2,
                          ),
                          decoration: BoxDecoration(
                            color: BragiColors.selection,
                            border: Border.all(color: BragiColors.border),
                            borderRadius: BorderRadius.circular(10),
                          ),
                          child: Text(e, style: const TextStyle(fontSize: 14)),
                        ),
                      ),
                  ],
                ),
              ),
            if (!mine && !streaming && message.text.isNotEmpty)
              Align(
                alignment: Alignment.centerRight,
                child: IconButton(
                  key: const Key('copy-message'),
                  tooltip: 'Copy as Markdown',
                  visualDensity: VisualDensity.compact,
                  iconSize: 16,
                  color: BragiColors.textSecondary,
                  onPressed: () =>
                      Clipboard.setData(ClipboardData(text: message.text)),
                  icon: const Icon(Icons.copy),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

IconData _kindIcon(String kind) => switch (kind) {
  'image' => Icons.image_outlined,
  'audio' => Icons.graphic_eq,
  _ => Icons.description_outlined,
};

/// Saves a file card's file. On a phone the system sheet takes the bytes and writes them where
/// the user picks; on a computer the save dialog returns a path and bragi-core writes it. The
/// scripted demo, with no one to answer a dialog, saves into a temp folder.
Future<void> saveFile(BragiController c, FileView f) async {
  if (_scripted) {
    await c.saveFileTo(
      f,
      Directory.systemTemp.createTempSync('bragi-saved').path,
    );
    return;
  }
  final mobile = Platform.isAndroid || Platform.isIOS;
  try {
    final bytes = mobile ? await c.fileBytes(f) : null;
    final dest = await FilePicker.platform.saveFile(
      dialogTitle: 'Save ${f.name}',
      fileName: f.name,
      bytes: bytes,
    );
    if (dest == null) return;
    if (mobile) {
      c.say('Saved ${f.name}');
    } else {
      await c.saveFileTo(f, dest);
    }
  } catch (e) {
    c.say('Could not save ${f.name}: $e');
  }
}

/// A file in the chat: its name, type and size, a preview (the first lines of a text file, or
/// the picture), and Save.
class _FileCard extends StatelessWidget {
  const _FileCard({required this.controller, required this.file});

  final BragiController controller;
  final FileView file;

  @override
  Widget build(BuildContext context) {
    final f = file;
    final from = f.origin == 'reply' ? 'from the answer' : 'attached';
    return Container(
      key: Key('file-${f.id}'),
      margin: const EdgeInsets.only(top: 6, bottom: 2),
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(
        color: BragiColors.background,
        border: Border.all(color: BragiColors.border),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(
                _kindIcon(f.kind),
                size: 18,
                color: BragiColors.textSecondary,
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      f.path ?? f.name,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 13),
                    ),
                    Text(
                      '${f.mime} · ${humanSize(f.size.toInt())} · $from',
                      style: TextStyle(
                        fontSize: 11,
                        color: BragiColors.textSecondary,
                      ),
                    ),
                  ],
                ),
              ),
              TextButton.icon(
                key: Key('save-${f.id}'),
                onPressed: () => saveFile(controller, f),
                icon: const Icon(Icons.download, size: 16),
                label: const Text('Save'),
              ),
            ],
          ),
          if (f.preview != null)
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Text(
                f.preview!,
                maxLines: 8,
                overflow: TextOverflow.fade,
                style: TextStyle(
                  fontFamily: 'monospace',
                  fontSize: 12,
                  color: BragiColors.textSecondary,
                ),
              ),
            ),
          if (f.kind == 'audio') _AudioRow(controller: controller, file: f),
          if (f.kind == 'image')
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: FutureBuilder(
                future: controller.fileBytes(f),
                builder: (_, snap) => snap.hasData
                    ? ConstrainedBox(
                        constraints: const BoxConstraints(maxHeight: 220),
                        child: Image.memory(
                          snap.data!,
                          key: Key('preview-${f.id}'),
                          fit: BoxFit.contain,
                          errorBuilder: (_, _, _) => const Text(
                            '(no preview)',
                            style: TextStyle(fontSize: 11),
                          ),
                        ),
                      )
                    : const SizedBox(height: 4),
              ),
            ),
        ],
      ),
    );
  }
}

/// A clip in the chat: the platform player inline (Play/Pause, a seek bar, elapsed / total,
/// Mute), or why it can't play here with the system player, then the frames the model saw (a tap
/// plays from there).
class _VideoCard extends StatelessWidget {
  const _VideoCard({
    required this.controller,
    required this.file,
    required this.info,
  });

  final BragiController controller;
  final String file;
  final VideoInfoView info;

  @override
  Widget build(BuildContext context) {
    final c = controller.video;
    final mine =
        c != null && c.file == file && c.conversation == controller.current?.id;
    final known = Duration(milliseconds: info.durationMs.toInt());
    final total = mine ? (c.duration ?? known) : known;
    final at = mine ? c.position : Duration.zero;
    final playing = mine && c.playing;
    final progress = total.inMilliseconds == 0
        ? 0.0
        : (at.inMilliseconds / total.inMilliseconds).clamp(0.0, 1.0);
    final fallback = controller.videoFallbacks[file];
    final picture = mine && !c.loading ? controller.videoView() : null;
    return Container(
      key: Key('video-$file'),
      margin: const EdgeInsets.only(top: 6),
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(
        border: Border.all(color: BragiColors.border),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            info.bubble,
            key: Key('vbubble-$file'),
            style: const TextStyle(fontSize: 13),
          ),
          const SizedBox(height: 6),
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 240),
            child:
                picture ??
                (info.frames.isEmpty
                    ? const SizedBox.shrink()
                    : Image.memory(
                        base64Decode(info.frames.first.data),
                        fit: BoxFit.contain,
                        gaplessPlayback: true,
                        errorBuilder: (_, _, _) => const SizedBox.shrink(),
                      )),
          ),
          if (fallback != null)
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Text(
                videoFallbackMessage(fallback.$1, fallback.$2),
                key: Key('vfallback-$file'),
                style: TextStyle(
                  fontSize: 12,
                  color: BragiColors.textSecondary,
                ),
              ),
            ),
          Row(
            children: [
              if (fallback == null) ...[
                IconButton(
                  key: Key('vplay-$file'),
                  tooltip: playing ? 'Pause' : 'Play',
                  visualDensity: VisualDensity.compact,
                  padding: EdgeInsets.zero,
                  constraints: const BoxConstraints(
                    minWidth: 32,
                    minHeight: 32,
                  ),
                  onPressed: () => controller.playVideo(file, info),
                  icon: Icon(
                    mine && c.loading
                        ? Icons.hourglass_empty
                        : (playing ? Icons.pause : Icons.play_arrow),
                  ),
                ),
                Expanded(
                  child: Slider(
                    key: Key('vseek-$file'),
                    value: progress,
                    onChanged: mine && !c.loading
                        ? (v) => controller.seekVideo(file, v)
                        : null,
                  ),
                ),
                Text(
                  rust.audioTimes(
                    positionMs: BigInt.from(at.inMilliseconds),
                    durationMs: BigInt.from(total.inMilliseconds),
                  ),
                  key: Key('vtime-$file'),
                  style: TextStyle(
                    fontSize: 12,
                    color: BragiColors.textSecondary,
                  ),
                ),
                if (mine)
                  IconButton(
                    key: Key('vmute-$file'),
                    tooltip: c.muted ? 'Unmute' : 'Mute',
                    visualDensity: VisualDensity.compact,
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints(
                      minWidth: 32,
                      minHeight: 32,
                    ),
                    onPressed: controller.toggleVideoMute,
                    icon: Icon(c.muted ? Icons.volume_off : Icons.volume_up),
                  ),
                if (info.clipHere)
                  IconButton(
                    key: Key('vopen-$file'),
                    tooltip: 'Open in system player',
                    visualDensity: VisualDensity.compact,
                    padding: EdgeInsets.zero,
                    constraints: const BoxConstraints(
                      minWidth: 32,
                      minHeight: 32,
                    ),
                    onPressed: () => controller.openVideoExternally(info),
                    icon: const Icon(Icons.open_in_new, size: 18),
                  ),
              ] else ...[
                const Spacer(),
                if (info.clipHere)
                  Flexible(
                    child: TextButton.icon(
                      key: Key('vopen-$file'),
                      onPressed: () => controller.openVideoExternally(info),
                      icon: const Icon(Icons.open_in_new, size: 16),
                      label: const Text(
                        'Open in system player',
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ),
              ],
            ],
          ),
          if (info.frames.isNotEmpty) ...[
            Text(
              'Frames the model saw',
              style: TextStyle(fontSize: 11, color: BragiColors.textSecondary),
            ),
            const SizedBox(height: 4),
            Wrap(
              spacing: 4,
              runSpacing: 4,
              children: [
                for (final (i, fr) in info.frames.indexed)
                  InkWell(
                    key: Key('vframe-$file-$i'),
                    onTap: () => controller.playVideo(
                      file,
                      info,
                      from: Duration(milliseconds: fr.atMs.toInt()),
                    ),
                    child: Column(
                      children: [
                        Image.memory(
                          base64Decode(fr.data),
                          width: 72,
                          height: 40,
                          fit: BoxFit.cover,
                          gaplessPlayback: true,
                          errorBuilder: (_, _, _) =>
                              const SizedBox(width: 72, height: 40),
                        ),
                        Text(fr.label, style: const TextStyle(fontSize: 10)),
                      ],
                    ),
                  ),
              ],
            ),
          ],
        ],
      ),
    );
  }
}

/// An audio file's player: Play/Pause, a seek bar and elapsed / total.
class _AudioRow extends StatelessWidget {
  const _AudioRow({required this.controller, required this.file});

  final BragiController controller;
  final FileView file;

  @override
  Widget build(BuildContext context) {
    final f = file;
    final c = controller.clip;
    final mine =
        c != null && c.file == f.id && c.conversation == controller.current?.id;
    final known = Duration(milliseconds: f.durationMs?.toInt() ?? 0);
    final total = mine ? (c.duration ?? known) : known;
    final at = mine ? c.position : Duration.zero;
    final playing = mine && c.playing;
    final progress = total.inMilliseconds == 0
        ? 0.0
        : (at.inMilliseconds / total.inMilliseconds).clamp(0.0, 1.0);
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: Row(
        children: [
          IconButton(
            key: Key('play-${f.id}'),
            tooltip: playing ? 'Pause' : 'Play',
            visualDensity: VisualDensity.compact,
            onPressed: () => controller.playFile(f),
            icon: Icon(
              mine && c.loading
                  ? Icons.hourglass_empty
                  : (playing ? Icons.pause : Icons.play_arrow),
            ),
          ),
          Expanded(
            child: Slider(
              key: Key('seek-${f.id}'),
              value: progress,
              onChanged: mine && !c.loading
                  ? (v) => controller.seekFile(f, v)
                  : null,
            ),
          ),
          Text(
            rust.audioTimes(
              positionMs: BigInt.from(at.inMilliseconds),
              durationMs: BigInt.from(total.inMilliseconds),
            ),
            key: Key('time-${f.id}'),
            style: TextStyle(fontSize: 12, color: BragiColors.textSecondary),
          ),
        ],
      ),
    );
  }
}

/// The collapsible Thinking section above an answer: the model's reasoning as plain text,
/// streaming live, with the time it took (the title comes from bragi-core, as on the desktop).
/// Open while the model reasons, closed once the answer starts; a tap flips it.
class _Thinking extends StatefulWidget {
  const _Thinking({
    required this.message,
    required this.streaming,
    required this.controller,
  });

  final MessageView message;
  final bool streaming;
  final BragiController controller;

  @override
  State<_Thinking> createState() => _ThinkingState();
}

class _ThinkingState extends State<_Thinking> {
  bool _flipped = false;
  Timer? _tick;

  bool get _reasoningNow =>
      widget.streaming &&
      widget.message.text.isEmpty &&
      widget.message.thinkingMs == null;

  @override
  void initState() {
    super.initState();
    _ticker();
  }

  @override
  void didUpdateWidget(_Thinking old) {
    super.didUpdateWidget(old);
    _ticker();
  }

  /// Redraws the elapsed time while the model reasons.
  void _ticker() {
    if (_reasoningNow) {
      _tick ??= Timer.periodic(
        const Duration(milliseconds: 500),
        (_) => setState(() {}),
      );
    } else {
      _tick?.cancel();
      _tick = null;
    }
  }

  @override
  void dispose() {
    _tick?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final now = _reasoningNow;
    final open = now != _flipped;
    final since = widget.controller.thinkingSince;
    final elapsed = now && since != null
        ? DateTime.now().difference(since).inMilliseconds
        : (widget.message.thinkingMs?.toInt() ?? 0);
    final title = widget.controller.api.thinkingLabel(elapsed, !now);
    return Padding(
      padding: const EdgeInsets.only(top: 2, bottom: 6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          InkWell(
            key: const Key('thinking'),
            onTap: () => setState(() => _flipped = !_flipped),
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 2),
              child: Text(
                '${open ? '▾' : '▸'} $title',
                style: TextStyle(
                  fontSize: 12,
                  color: BragiColors.textSecondary,
                ),
              ),
            ),
          ),
          if (open)
            Container(
              key: const Key('thinking-text'),
              margin: const EdgeInsets.only(top: 4),
              padding: const EdgeInsets.only(left: 10),
              decoration: BoxDecoration(
                border: Border(
                  left: BorderSide(color: BragiColors.border, width: 2),
                ),
              ),
              child: DefaultTextStyle.merge(
                style: TextStyle(color: BragiColors.textSecondary),
                child: MarkdownView(
                  key: const Key('thinking-markdown'),
                  doc: widget.controller.markdown(
                    widget.message.reasoning.trim(),
                    now,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _History extends StatelessWidget {
  const _History({
    required this.controller,
    required this.link,
    this.persistent = false,
  });

  final BragiController controller;
  final TextEditingController link;
  final bool persistent;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final check = c.linkCheck;
    void closeDrawer() {
      if (!persistent) Navigator.of(context).pop();
    }

    final content = SafeArea(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ListTile(
            leading: const Icon(Icons.add),
            title: const Text('New chat'),
            onTap: () {
              c.newChat();
              closeDrawer();
            },
          ),
          Padding(
            padding: EdgeInsets.fromLTRB(16, 8, 8, 4),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    'HISTORY',
                    style: TextStyle(
                      fontSize: 11,
                      color: BragiColors.textSecondary,
                    ),
                  ),
                ),
                if (persistent)
                  TextButton(
                    key: const Key('hide-sidebar'),
                    onPressed: c.toggleSidebar,
                    child: const Text('Hide'),
                  ),
              ],
            ),
          ),
          Expanded(child: SessionsTree(controller: c)),
          ListTile(
            key: const Key('models'),
            dense: true,
            leading: const Icon(Icons.memory_outlined),
            title: const Text('Models'),
            onTap: () {
              final nav = Navigator.of(context);
              closeDrawer();
              nav.push(
                MaterialPageRoute<void>(
                  builder: (_) => ModelsScreen(controller: c),
                ),
              );
            },
          ),
          ListTile(
            key: const Key('theme'),
            dense: true,
            leading: const Icon(Icons.palette_outlined),
            title: const Text('Theme'),
            onTap: () {
              final nav = Navigator.of(context);
              closeDrawer();
              nav.push(
                MaterialPageRoute<void>(
                  builder: (_) => ThemeScreen(appearance: c.appearance),
                ),
              );
            },
          ),
          ListTile(
            key: const Key('settings'),
            dense: true,
            leading: const Icon(Icons.settings_outlined),
            title: const Text('Settings'),
            onTap: () {
              final nav = Navigator.of(context);
              closeDrawer();
              c.reloadSettings();
              nav.push(
                MaterialPageRoute<void>(
                  builder: (_) => SettingsScreen(controller: c),
                ),
              );
            },
          ),
          const Divider(),
          Padding(
            padding: const EdgeInsets.all(12),
            child: check != null
                ? Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('${check.device} shows ${check.sas}?'),
                      const SizedBox(height: 8),
                      Row(
                        children: [
                          FilledButton(
                            key: const Key('link-yes'),
                            onPressed: () => c.linkConfirm(true),
                            child: const Text('Match'),
                          ),
                          const SizedBox(width: 8),
                          OutlinedButton(
                            key: const Key('link-no'),
                            onPressed: () => c.linkConfirm(false),
                            child: const Text('Cancel'),
                          ),
                        ],
                      ),
                    ],
                  )
                : Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text(
                        'LINK A DEVICE',
                        style: TextStyle(
                          fontSize: 11,
                          color: BragiColors.textSecondary,
                        ),
                      ),
                      const SizedBox(height: 6),
                      TextField(
                        key: const Key('link-code'),
                        controller: link,
                        decoration: const InputDecoration(
                          hintText: 'Link code or node address',
                          hintMaxLines: 1,
                          hintStyle: TextStyle(overflow: TextOverflow.ellipsis),
                          isDense: true,
                          border: OutlineInputBorder(),
                        ),
                      ),
                      const SizedBox(height: 6),
                      OutlinedButton(
                        key: const Key('link-go'),
                        onPressed: () async {
                          if (await c.linkDevice(link.text)) link.clear();
                        },
                        child: const Text('Link'),
                      ),
                    ],
                  ),
          ),
        ],
      ),
    );
    if (persistent) {
      return ColoredBox(
        color:
            Theme.of(context).drawerTheme.backgroundColor ??
            Theme.of(context).colorScheme.surface,
        child: content,
      );
    }
    return Drawer(child: content);
  }
}

/// A chip or bubble line for an attachment: a text file by its length, an image or sound by its kind.
String _attachmentLabel(AttachmentView a) => switch (a.kind) {
  'image' => '${a.name} (image)',
  'audio' => '${a.name} (audio)',
  'video' => '${a.name} (video)',
  _ => '${a.name} (${a.text.trimRight().split('\n').length} lines)',
};
