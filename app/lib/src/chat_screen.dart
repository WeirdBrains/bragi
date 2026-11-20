// The chat: pick a model from the catalog, ask, watch it stream, stop it, continue it.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
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

  @override
  void dispose() {
    _prompt.dispose();
    _link.dispose();
    _scroll.dispose();
    super.dispose();
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
      if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
    return Scaffold(
      appBar: AppBar(
        title: Text(c.current?.title ?? 'Bragi', overflow: TextOverflow.ellipsis),
        actions: [
          IconButton(key: const Key('new-chat'), tooltip: 'New chat', onPressed: c.newChat, icon: const Icon(Icons.add_comment_outlined)),
        ],
        bottom: PreferredSize(preferredSize: const Size.fromHeight(48), child: _Models(controller: c)),
      ),
      drawer: _History(controller: c, link: _link),
      body: SafeArea(
        child: Column(children: [
          Expanded(
            child: c.messages.isEmpty
                ? const Center(child: Text('Ask your own cloud something.', style: TextStyle(color: BragiColors.textSecondary)))
                : ListView.builder(
                    key: const Key('messages'),
                    controller: _scroll,
                    padding: const EdgeInsets.all(12),
                    itemCount: c.messages.length,
                    itemBuilder: (_, i) => _Bubble(message: c.messages[i], streaming: c.streaming && i == c.messages.length - 1),
                  ),
          ),
          if (c.notice != null)
            Padding(padding: const EdgeInsets.all(6), child: Text(c.notice!, style: const TextStyle(color: BragiColors.error, fontSize: 12))),
          Container(
            decoration: const BoxDecoration(border: Border(top: BorderSide(color: BragiColors.border))),
            padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
            child: Row(children: [
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
                  ? OutlinedButton.icon(key: const Key('stop'), onPressed: c.stop, icon: const Icon(Icons.stop, size: 16), label: const Text('Stop'))
                  : FilledButton(key: const Key('send'), onPressed: _send, child: const Text('Send')),
            ]),
          ),
        ]),
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
        child: Row(children: [
          for (final m in c.models)
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: ChoiceChip(
                key: Key('model-${m.id}'),
                selected: c.model == m.id,
                onSelected: (_) => c.pickModel(m.id),
                avatar: CircleAvatar(radius: 4, backgroundColor: _dot(m.status)),
                label: Text('${m.id} · ${_label(m)}'),
              ),
            ),
        ]),
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
    final s = switch (m.status) { 'loadable' => 'loads on use', final s => s };
    final speed = m.tokensPerSec == null ? '' : ' · ${m.tokensPerSec!.round()} tok/s';
    return '$s$speed';
  }
}

class _Bubble extends StatelessWidget {
  const _Bubble({required this.message, required this.streaming});

  final MessageView message;
  final bool streaming;

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
    return Align(
      alignment: mine ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        constraints: const BoxConstraints(maxWidth: 560),
        margin: const EdgeInsets.symmetric(vertical: 5),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(color: mine ? BragiColors.selection : BragiColors.surface, borderRadius: BorderRadius.circular(12)),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          if (meta != null && meta.isNotEmpty) Text(meta, style: const TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
          Text(message.text.isEmpty ? '…' : message.text, style: const TextStyle(fontSize: 15, height: 1.4)),
        ]),
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
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          ListTile(
            leading: const Icon(Icons.add),
            title: const Text('New chat'),
            onTap: () {
              c.newChat();
              Navigator.of(context).pop();
            },
          ),
          const Padding(padding: EdgeInsets.fromLTRB(16, 8, 16, 4), child: Text('HISTORY', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary))),
          Expanded(
            child: ListView(children: [
              for (final s in c.conversations)
                ListTile(
                  key: Key('conv-${s.id}'),
                  dense: true,
                  selected: c.current?.id == s.id,
                  title: Text(s.title, overflow: TextOverflow.ellipsis),
                  onTap: () {
                    c.open(s.id);
                    Navigator.of(context).pop();
                  },
                ),
            ]),
          ),
          const Divider(),
          Padding(
            padding: const EdgeInsets.all(12),
            child: check != null
                ? Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    Text('${check.device} shows ${check.sas}?'),
                    const SizedBox(height: 8),
                    Row(children: [
                      FilledButton(key: const Key('link-yes'), onPressed: () => c.linkConfirm(true), child: const Text('Match')),
                      const SizedBox(width: 8),
                      OutlinedButton(key: const Key('link-no'), onPressed: () => c.linkConfirm(false), child: const Text('Cancel')),
                    ]),
                  ])
                : Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                    const Text('LINK A DEVICE', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
                    const SizedBox(height: 6),
                    TextField(
                      key: const Key('link-code'),
                      controller: link,
                      decoration: const InputDecoration(hintText: 'weirdnet-bragi:// code', isDense: true, border: OutlineInputBorder()),
                    ),
                    const SizedBox(height: 6),
                    OutlinedButton(key: const Key('link-go'), onPressed: () => c.linkDevice(link.text), child: const Text('Link')),
                  ]),
          ),
        ]),
      ),
    );
  }
}
