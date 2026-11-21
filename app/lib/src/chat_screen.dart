// The chat: pick a model from the catalog, ask, watch it stream, stop it, continue it.
import 'dart:async';
import 'dart:convert';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'markdown.dart';
import 'delete_ui.dart';
import 'theme.dart';

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
  }

  void _onScroll() {
    if (!_scroll.hasClients) return;
    final atBottom = _scroll.position.maxScrollExtent - _scroll.position.pixels <= 24;
    if (atBottom != _stick) setState(() => _stick = atBottom);
  }

  void _jumpToLatest() {
    setState(() => _stick = true);
    if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
  }

  @override
  void dispose() {
    _prompt.dispose();
    _link.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// The system picker; the scripted demo (no one to tap a system dialog) attaches a sample file.
  Future<void> _attach() async {
    final c = widget.controller;
    if (_scripted) {
      c.attachFile('sample-notes.md', utf8.encode('# Notes\n- buy milk\n- write the B5 milestone\n'));
      return;
    }
    final r = await FilePicker.platform.pickFiles(withData: true);
    for (final f in r?.files ?? const <PlatformFile>[]) {
      final bytes = f.bytes;
      if (bytes != null) c.attachFile(f.name, bytes);
    }
  }

  void _send() {
    final text = _prompt.text;
    _prompt.clear();
    widget.controller.send(text);
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.controller;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_stick && _scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
    return Scaffold(
      appBar: AppBar(
        title: Text(c.current?.title ?? 'Bragi', overflow: TextOverflow.ellipsis),
        actions: [
          IconButton(key: const Key('new-chat'), tooltip: 'New chat', onPressed: c.newChat, icon: const Icon(Icons.add_comment_outlined)),
        ],
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(48),
          child: _Models(controller: c),
        ),
      ),
      drawer: _History(controller: c, link: _link),
      body: SafeArea(
        child: Column(
          children: [
            Expanded(
              child: c.messages.isEmpty
                  ? const Center(
                      child: Text('Ask your own cloud something.', style: TextStyle(color: BragiColors.textSecondary)),
                    )
                  // Replies stay selectable; each reply's copy button gives its raw Markdown.
                  : Stack(
                      children: [
                        SelectionArea(child: ListView.builder(
                          key: const Key('messages'),
                          controller: _scroll,
                          padding: const EdgeInsets.all(12),
                          itemCount: c.messages.length,
                          itemBuilder: (context, i) => GestureDetector(
                            key: Key('bubble-$i'),
                            // Long-press (or right-click) a message for its menu; not while an answer streams.
                            onLongPress: c.streaming ? null : () => messageMenu(context, c, c.messages[i]),
                            onSecondaryTap: c.streaming ? null : () => messageMenu(context, c, c.messages[i]),
                            child: _Bubble(key: ValueKey('bubble-${c.current?.id}-$i'), message: c.messages[i], streaming: c.streaming && i == c.messages.length - 1, controller: c),
                          ),
                        )),
                        if (!_stick)
                          Positioned(
                            bottom: 8,
                            left: 0,
                            right: 0,
                            child: Center(
                              child: FilledButton.icon(
                                key: const Key('jump-latest'),
                                onPressed: _jumpToLatest,
                                icon: const Icon(Icons.arrow_downward, size: 16),
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
                child: Text(c.notice!, style: const TextStyle(color: BragiColors.error, fontSize: 12)),
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
            if (c.attachments.isNotEmpty)
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 4, 10, 0),
                child: Wrap(
                  spacing: 6,
                  children: [
                    for (final (i, a) in c.attachments.indexed)
                      InputChip(
                        key: Key('chip-$i'),
                        label: Text('${a.name} (${a.text.trimRight().split('\n').length} lines)'),
                        onDeleted: () => c.detach(i),
                      ),
                  ],
                ),
              ),
            Container(
              decoration: const BoxDecoration(
                border: Border(top: BorderSide(color: BragiColors.border)),
              ),
              padding: const EdgeInsets.fromLTRB(4, 8, 10, 8),
              child: Row(
                children: [
                  IconButton(
                    key: const Key('attach'),
                    tooltip: 'Attach a text file',
                    onPressed: _attach,
                    icon: const Icon(Icons.attach_file),
                  ),
                  IconButton(
                    key: const Key('agent'),
                    tooltip: c.agent ? 'Agent mode: on' : 'Agent mode: off',
                    isSelected: c.agent,
                    style: c.agent
                        ? IconButton.styleFrom(backgroundColor: BragiColors.accent, foregroundColor: BragiColors.background)
                        : null,
                    onPressed: c.toggleAgent,
                    icon: const Icon(Icons.smart_toy_outlined),
                  ),
                  const SizedBox(width: 6),
                  Expanded(
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
                      decoration: const InputDecoration(hintText: 'Ask anything…', isDense: true, border: OutlineInputBorder()),
                    ),
                  ),
                  if (c.canContinue) ...[
                    const SizedBox(width: 6),
                    OutlinedButton(key: const Key('continue'), onPressed: c.continueAnswer, child: const Text('Continue')),
                  ],
                  const SizedBox(width: 6),
                  c.streaming
                      ? OutlinedButton.icon(
                          key: const Key('stop'),
                          onPressed: c.stop,
                          icon: const Icon(Icons.stop, size: 16),
                          label: const Text('Stop'),
                        )
                      : FilledButton(key: const Key('send'), onPressed: _send, child: const Text('Send')),
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
            style: const TextStyle(fontSize: 11, color: BragiColors.textSecondary),
          ),
          const SizedBox(height: 8),
          Wrap(
            spacing: 8,
            children: [
              FilledButton(key: const Key('allow-once'), onPressed: () => c.answer('once'), child: const Text('Allow once')),
              OutlinedButton(key: const Key('allow-task'), onPressed: () => c.answer('task'), child: const Text('This task')),
              OutlinedButton(key: const Key('deny'), onPressed: () => c.answer('deny'), child: const Text('Deny')),
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
      return const Padding(
        padding: EdgeInsets.all(12),
        child: Text('Looking for your inference nodes…', style: TextStyle(color: BragiColors.textSecondary)),
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
                  avatar: CircleAvatar(radius: 4, backgroundColor: _dot(m.status)),
                  label: Text('${m.id} · ${_label(m)}${c.state!.nodeCount > 1 ? ' · ${m.nodes}/${c.state!.nodeCount} nodes' : ''}'),
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
    final speed = m.tokensPerSec == null ? '' : ' · ${m.tokensPerSec!.round()} tok/s';
    return '$s$speed';
  }
}

class _Bubble extends StatelessWidget {
  const _Bubble({super.key, required this.message, required this.streaming, required this.controller});

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
      if (message.resumes == 1) 'moved to another node' else if (message.resumes > 1) 'moved nodes ${message.resumes}×',
      if (message.agent) message.tools.isEmpty ? 'agent task' : 'agent task · used ${message.tools.join(', ')}',
    ];
    final line = mine ? null : [if (meta != null && meta.isNotEmpty) meta, ...flags].join(' · ');
    return Align(
      alignment: mine ? Alignment.centerRight : Alignment.centerLeft,
      child: GestureDetector(
        key: const Key('bubble'),
        onLongPress: () => _bar(context),
        child: Container(
        constraints: const BoxConstraints(maxWidth: 560),
        margin: const EdgeInsets.symmetric(vertical: 5),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(color: mine ? BragiColors.selection : BragiColors.surface, borderRadius: BorderRadius.circular(12)),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final a in message.attachments)
              Padding(
                padding: const EdgeInsets.only(bottom: 4),
                child: Text(
                  'attached: ${a.name} (${a.text.trimRight().split('\n').length} lines)',
                  key: const Key('bubble-attachment'),
                  style: const TextStyle(fontSize: 12, color: BragiColors.textSecondary),
                ),
              ),
            if (line != null && line.isNotEmpty)
              Text(
                line,
                key: const Key('bubble-meta'),
                style: const TextStyle(fontSize: 11, color: BragiColors.textSecondary),
              ),
            if (!mine && message.reasoning.trim().isNotEmpty) _Thinking(message: message, streaming: streaming, controller: controller),
            if (mine || message.text.isEmpty)
              Text(
                message.text.isEmpty
                    // Nothing from the model yet: say so, until reasoning or text arrives.
                    ? (streaming && message.reasoning.trim().isEmpty ? 'Waiting for a reply…' : '…')
                    : message.text,
                key: message.text.isEmpty && streaming ? const Key('bubble-waiting') : null,
                style: const TextStyle(fontSize: 15, height: 1.4),
              )
            else
              MarkdownView(key: const Key('bubble-markdown'), doc: controller.markdown(message.text, streaming || message.state == 'streaming')),
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
                        onTap: streaming ? null : () => controller.react(message, e),
                        child: Container(
                          padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
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
                  onPressed: () => Clipboard.setData(ClipboardData(text: message.text)),
                  icon: const Icon(Icons.copy),
                ),
              ),
          ],
        ),
      ),
      ),
    );
  }
}

/// The collapsible Thinking section above an answer: the model's reasoning as plain text,
/// streaming live, with the time it took (the title comes from bragi-core, as on the desktop).
/// Open while the model reasons, closed once the answer starts; a tap flips it.
class _Thinking extends StatefulWidget {
  const _Thinking({required this.message, required this.streaming, required this.controller});

  final MessageView message;
  final bool streaming;
  final BragiController controller;

  @override
  State<_Thinking> createState() => _ThinkingState();
}

class _ThinkingState extends State<_Thinking> {
  bool _flipped = false;
  Timer? _tick;

  bool get _reasoningNow => widget.streaming && widget.message.text.isEmpty && widget.message.thinkingMs == null;

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
      _tick ??= Timer.periodic(const Duration(milliseconds: 500), (_) => setState(() {}));
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
    final elapsed = now && since != null ? DateTime.now().difference(since).inMilliseconds : (widget.message.thinkingMs?.toInt() ?? 0);
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
              child: Text('${open ? '▾' : '▸'} $title', style: const TextStyle(fontSize: 12, color: BragiColors.textSecondary)),
            ),
          ),
          if (open)
            Container(
              key: const Key('thinking-text'),
              margin: const EdgeInsets.only(top: 4),
              padding: const EdgeInsets.only(left: 10),
              decoration: const BoxDecoration(border: Border(left: BorderSide(color: BragiColors.border, width: 2))),
              child: DefaultTextStyle.merge(
                style: const TextStyle(color: BragiColors.textSecondary),
                child: MarkdownView(key: const Key('thinking-markdown'), doc: widget.controller.markdown(widget.message.reasoning.trim(), now)),
              ),
            ),
        ],
      ),
    );
  }
}

class _History extends StatelessWidget {
  const _History({required this.controller, required this.link});

  final BragiController controller;
  final TextEditingController link;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final check = c.linkCheck;
    return Drawer(
      child: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ListTile(
              leading: const Icon(Icons.add),
              title: const Text('New chat'),
              onTap: () {
                c.newChat();
                Navigator.of(context).pop();
              },
            ),
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 8, 16, 4),
              child: Text('HISTORY', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
            ),
            Expanded(
              child: ListView(
                children: [
                  for (final s in c.conversations)
                    ListTile(
                      key: Key('conv-${s.id}'),
                      dense: true,
                      selected: c.current?.id == s.id,
                      title: Text(s.title, overflow: TextOverflow.ellipsis),
                      trailing: IconButton(
                        key: Key('delete-conv-${s.id}'),
                        tooltip: 'Delete conversation',
                        visualDensity: VisualDensity.compact,
                        icon: const Icon(Icons.delete_outline, size: 18),
                        onPressed: () => confirmDeleteConversation(context, c, s),
                      ),
                      onLongPress: () => confirmDeleteConversation(context, c, s),
                      onTap: () {
                        c.open(s.id);
                        Navigator.of(context).pop();
                      },
                    ),
                ],
              ),
            ),
            ListTile(
              key: const Key('settings'),
              dense: true,
              leading: const Icon(Icons.settings_outlined),
              title: const Text('Settings'),
              onTap: () {
                final nav = Navigator.of(context);
                nav.pop();
                nav.push(MaterialPageRoute<void>(builder: (_) => SettingsScreen(controller: c)));
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
                            FilledButton(key: const Key('link-yes'), onPressed: () => c.linkConfirm(true), child: const Text('Match')),
                            const SizedBox(width: 8),
                            OutlinedButton(key: const Key('link-no'), onPressed: () => c.linkConfirm(false), child: const Text('Cancel')),
                          ],
                        ),
                      ],
                    )
                  : Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        const Text('LINK A DEVICE', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
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
      ),
    );
  }
}
