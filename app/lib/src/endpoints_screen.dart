// Settings for where Bragi sends: OpenAI-compatible endpoints (a Ymir bridge can be the Weirdnet
// session's fallback) and Weirdnet links (the node addresses this device dials). Everything can be
// edited and deleted here. Edits are tested before they are saved; deletes ask first and never
// delete history. A token is typed, tested and kept by the core in a private file; it is never
// shown again, only replaced.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'theme.dart';

String kindName(String kind) => kind == 'ymir_bridge' ? 'Ymir bridge' : 'OpenAI-compatible';

String _testLine(EndpointTestView t) => '${kindName(t.kind)} · ${t.models.length} model${t.models.length == 1 ? '' : 's'}: ${t.models.join(', ')}';

class EndpointsScreen extends StatefulWidget {
  const EndpointsScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  State<EndpointsScreen> createState() => _EndpointsScreenState();
}

class _EndpointsScreenState extends State<EndpointsScreen> {
  final _name = TextEditingController();
  final _url = TextEditingController();
  final _token = TextEditingController();
  final _linkName = TextEditingController();
  final _linkAddress = TextEditingController();
  bool _busy = false;
  String? _tested;

  @override
  void dispose() {
    for (final c in [_name, _url, _token, _linkName, _linkAddress]) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _run(Future<void> Function() f) async {
    setState(() => _busy = true);
    await f();
    if (mounted) setState(() => _busy = false);
  }

  Future<void> _add() => _run(() async {
    final c = widget.controller;
    await c.addEndpoint(_name.text, _url.text, _token.text);
    if (c.notice == null) {
      _name.clear();
      _url.clear();
      _tested = null;
    }
    // The token never stays in a field.
    _token.clear();
  });

  Future<void> _testNew() => _run(() async {
    final t = await widget.controller.testEndpoint(null, _url.text, _token.text);
    _tested = t == null ? null : _testLine(t);
  });

  Future<void> _addLink() => _run(() async {
    final c = widget.controller;
    if (await c.addLink(_linkName.text, _linkAddress.text)) {
      _linkName.clear();
      _linkAddress.clear();
    }
  });

  static String _health(EndpointView e) => switch (e.health) {
    'ok' => '${e.models.length} model${e.models.length == 1 ? '' : 's'}${e.defaultModel != null ? ' · default ${e.defaultModel}' : ''}',
    'failed' => 'not answering: ${e.message ?? ''}',
    _ => 'checking…',
  };

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final c = widget.controller;
        const label = TextStyle(fontSize: 11, color: BragiColors.textSecondary);
        return Scaffold(
          appBar: AppBar(title: const Text('Endpoints and links')),
          body: ListView(
            padding: const EdgeInsets.all(12),
            children: [
              if (c.route case final r?)
                Card(
                  child: ListTile(
                    key: const Key('endpoints-route'),
                    leading: Icon(r.kind == 'fallback' ? Icons.alt_route : Icons.route, color: r.kind == 'fallback' ? BragiColors.busy : null),
                    title: const Text('Where this session sends'),
                    subtitle: Text(r.detail),
                  ),
                ),
              if (c.notice != null)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 6),
                  child: Text(c.notice!, key: const Key('settings-notice'), style: const TextStyle(color: BragiColors.error, fontSize: 12)),
                ),
              const Text('ENDPOINTS', style: label),
              for (final e in c.endpoints)
                Card(
                  key: Key('endpoint-${e.id}'),
                  child: Padding(
                    padding: const EdgeInsets.all(8),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        ListTile(
                          contentPadding: EdgeInsets.zero,
                          leading: CircleAvatar(
                            radius: 5,
                            backgroundColor: switch (e.health) {
                              'ok' => BragiColors.warm,
                              'failed' => BragiColors.error,
                              _ => BragiColors.textSecondary,
                            },
                          ),
                          minLeadingWidth: 10,
                          title: Text(e.name),
                          subtitle: Text(
                            '${kindName(e.kind)} · ${e.url}\n${_health(e)}${e.sessions > 0 ? ' · ${e.sessions} session${e.sessions == 1 ? '' : 's'}' : ''}',
                          ),
                          isThreeLine: true,
                        ),
                        SwitchListTile(
                          key: Key('endpoint-fallback-${e.id}'),
                          contentPadding: EdgeInsets.zero,
                          dense: true,
                          value: e.fallback,
                          title: const Text('Fallback for the Weirdnet session'),
                          subtitle: Text(
                            e.kind == 'ymir_bridge' ? 'Used only when no inference node answers' : 'Only a Ymir bridge can be the fallback',
                          ),
                          onChanged: e.kind == 'ymir_bridge' ? (on) => c.setFallback(on ? e.id : null) : null,
                        ),
                        Row(
                          mainAxisAlignment: MainAxisAlignment.end,
                          children: [
                            TextButton.icon(
                              key: Key('endpoint-edit-${e.id}'),
                              icon: const Icon(Icons.edit_outlined, size: 18),
                              label: const Text('Edit'),
                              onPressed: () => Navigator.of(context).push(
                                MaterialPageRoute<void>(builder: (_) => EditEndpointScreen(controller: c, endpoint: e)),
                              ),
                            ),
                            TextButton.icon(
                              key: Key('endpoint-remove-${e.id}'),
                              icon: const Icon(Icons.delete_outline, size: 18),
                              label: const Text('Delete'),
                              onPressed: () => confirmDeleteEndpoint(context, c, e),
                            ),
                          ],
                        ),
                      ],
                    ),
                  ),
                ),
              const SizedBox(height: 8),
              const Text('ADD AN ENDPOINT', style: label),
              TextField(key: const Key('endpoint-name'), controller: _name, decoration: const InputDecoration(labelText: 'Name', isDense: true)),
              TextField(
                key: const Key('endpoint-url'),
                controller: _url,
                keyboardType: TextInputType.url,
                autocorrect: false,
                decoration: const InputDecoration(labelText: 'URL', hintText: 'https://ymir.example.ts.net:9443/v1', isDense: true),
              ),
              TextField(
                key: const Key('endpoint-token'),
                controller: _token,
                obscureText: true,
                autocorrect: false,
                enableSuggestions: false,
                decoration: const InputDecoration(labelText: 'Token', isDense: true),
              ),
              if (_tested != null) Padding(padding: const EdgeInsets.only(top: 6), child: Text(_tested!, key: const Key('endpoint-tested'), style: label)),
              const SizedBox(height: 8),
              Row(
                children: [
                  OutlinedButton(key: const Key('endpoint-test'), onPressed: _busy ? null : _testNew, child: const Text('Test connection')),
                  const SizedBox(width: 8),
                  FilledButton(key: const Key('endpoint-add'), onPressed: _busy ? null : _add, child: Text(_busy ? 'Checking…' : 'Check and add')),
                ],
              ),
              const Divider(height: 32),
              const Text('WEIRDNET LINKS', style: label),
              const Padding(
                padding: EdgeInsets.only(top: 2, bottom: 4),
                child: Text('Node addresses this device dials. Sessions use the whole pool, so no session depends on one link.', style: label),
              ),
              for (final l in c.links)
                Card(
                  key: Key('link-${l.address}'),
                  child: ListTile(
                    leading: CircleAvatar(radius: 5, backgroundColor: l.connected ? BragiColors.warm : BragiColors.textSecondary),
                    minLeadingWidth: 10,
                    title: Text(l.name),
                    subtitle: Text('${l.address}${l.fixed ? '\nfrom Bragi\'s config: rename only' : ''}', style: const TextStyle(fontSize: 11)),
                    isThreeLine: l.fixed,
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        IconButton(
                          key: Key('link-edit-${l.address}'),
                          tooltip: 'Edit',
                          icon: const Icon(Icons.edit_outlined, size: 18),
                          onPressed: () => editLink(context, c, l),
                        ),
                        if (!l.fixed)
                          IconButton(
                            key: Key('link-delete-${l.address}'),
                            tooltip: 'Delete',
                            icon: const Icon(Icons.delete_outline, size: 18),
                            onPressed: () => confirmDeleteLink(context, c, l),
                          ),
                      ],
                    ),
                  ),
                ),
              TextField(key: const Key('link-name'), controller: _linkName, decoration: const InputDecoration(labelText: 'Name', isDense: true)),
              TextField(
                key: const Key('link-address'),
                controller: _linkAddress,
                autocorrect: false,
                decoration: const InputDecoration(labelText: 'Node address', hintText: '/ip4/…/tcp/4848/p2p/…', isDense: true),
              ),
              const SizedBox(height: 8),
              Align(
                alignment: Alignment.centerLeft,
                child: FilledButton(key: const Key('link-add'), onPressed: _busy ? null : _addLink, child: const Text('Test and add')),
              ),
            ],
          ),
        );
      },
    );
  }
}

/// Edit an endpoint: name, URL, a replacement token (the saved one is never shown), the default
/// model and the fallback. Test connection checks without saving; Save tests a changed URL or
/// token first and changes nothing if that fails.
class EditEndpointScreen extends StatefulWidget {
  const EditEndpointScreen({super.key, required this.controller, required this.endpoint});

  final BragiController controller;
  final EndpointView endpoint;

  @override
  State<EditEndpointScreen> createState() => _EditEndpointScreenState();
}

class _EditEndpointScreenState extends State<EditEndpointScreen> {
  late final _name = TextEditingController(text: widget.endpoint.name);
  late final _url = TextEditingController(text: widget.endpoint.url);
  final _token = TextEditingController();
  late String? _model = widget.endpoint.model;
  late bool _fallback = widget.endpoint.fallback;
  late List<String> _models = widget.endpoint.models;
  late String _kind = widget.endpoint.kind;
  String? _tested;
  bool _busy = false;

  @override
  void dispose() {
    _name.dispose();
    _url.dispose();
    _token.dispose();
    super.dispose();
  }

  Future<void> _test() async {
    setState(() => _busy = true);
    final t = await widget.controller.testEndpoint(widget.endpoint.id, _url.text, _token.text);
    if (!mounted) return;
    setState(() {
      _busy = false;
      _tested = t == null ? null : _testLine(t);
      if (t != null) {
        _models = t.models;
        _kind = t.kind;
        if (_model != null && !_models.contains(_model)) _model = null;
        if (_kind != 'ymir_bridge') _fallback = false;
      }
    });
  }

  Future<void> _save() async {
    setState(() => _busy = true);
    final ok = await widget.controller.editEndpoint(
      widget.endpoint.id,
      name: _name.text,
      url: _url.text,
      token: _token.text.isEmpty ? null : _token.text,
      model: _model,
      fallback: _fallback,
    );
    _token.clear();
    if (!mounted) return;
    setState(() => _busy = false);
    if (ok) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    const label = TextStyle(fontSize: 11, color: BragiColors.textSecondary);
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) => Scaffold(
        appBar: AppBar(title: Text('Edit ${widget.endpoint.name}')),
        body: ListView(
          padding: const EdgeInsets.all(16),
          children: [
            TextField(key: const Key('edit-name'), controller: _name, decoration: const InputDecoration(labelText: 'Name')),
            TextField(
              key: const Key('edit-url'),
              controller: _url,
              keyboardType: TextInputType.url,
              autocorrect: false,
              decoration: const InputDecoration(labelText: 'URL'),
            ),
            TextField(
              key: const Key('edit-token'),
              controller: _token,
              obscureText: true,
              autocorrect: false,
              enableSuggestions: false,
              decoration: const InputDecoration(labelText: 'New token', hintText: 'Saved token kept unless you type a new one'),
            ),
            const SizedBox(height: 8),
            DropdownButtonFormField<String?>(
              key: const Key('edit-model'),
              initialValue: _models.contains(_model) ? _model : null,
              decoration: const InputDecoration(labelText: 'Default model'),
              items: [
                const DropdownMenuItem<String?>(value: null, child: Text('The first one it lists')),
                for (final m in _models) DropdownMenuItem<String?>(value: m, child: Text(m)),
              ],
              onChanged: (v) => setState(() => _model = v),
            ),
            SwitchListTile(
              key: const Key('edit-fallback'),
              contentPadding: EdgeInsets.zero,
              value: _fallback,
              title: const Text('Fallback for the Weirdnet session'),
              subtitle: Text(_kind == 'ymir_bridge' ? 'Used only when no inference node answers' : 'Only a Ymir bridge can be the fallback'),
              onChanged: _kind == 'ymir_bridge' ? (v) => setState(() => _fallback = v) : null,
            ),
            if (_tested != null) Text(_tested!, key: const Key('edit-tested'), style: label),
            if (widget.controller.notice != null)
              Padding(
                padding: const EdgeInsets.only(top: 6),
                child: Text(widget.controller.notice!, key: const Key('edit-notice'), style: const TextStyle(color: BragiColors.error, fontSize: 12)),
              ),
            const SizedBox(height: 12),
            Row(
              children: [
                OutlinedButton(key: const Key('edit-test'), onPressed: _busy ? null : _test, child: const Text('Test connection')),
                const SizedBox(width: 8),
                FilledButton(key: const Key('edit-save'), onPressed: _busy ? null : _save, child: Text(_busy ? 'Testing…' : 'Save')),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// "Delete this endpoint?": says how many sessions send to it and asks where they go (Weirdnet,
/// another endpoint, or nowhere until picked). No conversation is deleted.
Future<void> confirmDeleteEndpoint(BuildContext context, BragiController c, EndpointView e) async {
  var rebind = e.sessions > 0 ? 'weirdnet' : 'unbound';
  final others = c.endpoints.where((x) => x.id != e.id).toList();
  final yes = await showDialog<bool>(
    context: context,
    builder: (ctx) => StatefulBuilder(
      builder: (ctx, set) => AlertDialog(
        title: Text('Delete ${e.name}?'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              e.sessions == 0
                  ? 'No session sends to it. Its saved token is deleted with it.'
                  : '${e.sessions} session${e.sessions == 1 ? ' sends' : 's send'} to it. Their conversations are kept. Send ${e.sessions == 1 ? 'it' : 'them'} to:',
              key: const Key('delete-endpoint-warning'),
            ),
            if (e.sessions > 0) ...[
              RadioGroup<String>(
                groupValue: rebind,
                onChanged: (v) => set(() => rebind = v ?? rebind),
                child: Column(
                  children: [
                    const RadioListTile<String>(key: Key('rebind-weirdnet'), dense: true, value: 'weirdnet', title: Text('Weirdnet')),
                    for (final o in others) RadioListTile<String>(key: Key('rebind-${o.id}'), dense: true, value: o.id, title: Text(o.name)),
                    const RadioListTile<String>(
                      key: Key('rebind-unbound'),
                      dense: true,
                      value: 'unbound',
                      title: Text('Nowhere for now'),
                      subtitle: Text('They say so until you pick a target'),
                    ),
                  ],
                ),
              ),
            ],
          ],
        ),
        actions: [
          TextButton(onPressed: () => Navigator.of(ctx).pop(false), child: const Text('Cancel')),
          FilledButton(key: const Key('delete-endpoint-ok'), onPressed: () => Navigator.of(ctx).pop(true), child: const Text('Delete')),
        ],
      ),
    ),
  );
  if (yes == true) await c.deleteEndpoint(e.id, rebind);
}

/// Edit a link: its name, and its address after a test connection.
Future<void> editLink(BuildContext context, BragiController c, LinkView l) async {
  final name = TextEditingController(text: l.name);
  final address = TextEditingController(text: l.address);
  String? tested;
  await showDialog<void>(
    context: context,
    builder: (ctx) => StatefulBuilder(
      builder: (ctx, set) => AlertDialog(
        title: const Text('Edit link'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(key: const Key('link-edit-name'), controller: name, decoration: const InputDecoration(labelText: 'Name')),
            TextField(
              key: const Key('link-edit-address'),
              controller: address,
              enabled: !l.fixed,
              autocorrect: false,
              decoration: const InputDecoration(labelText: 'Node address'),
            ),
            if (tested != null) Padding(padding: const EdgeInsets.only(top: 6), child: Text(tested!, style: const TextStyle(fontSize: 11))),
            if (c.notice != null)
              Padding(padding: const EdgeInsets.only(top: 6), child: Text(c.notice!, style: const TextStyle(color: BragiColors.error, fontSize: 12))),
          ],
        ),
        actions: [
          TextButton(onPressed: () => Navigator.of(ctx).pop(), child: const Text('Cancel')),
          if (!l.fixed)
            OutlinedButton(
              key: const Key('link-edit-test'),
              onPressed: () async {
                final peer = await c.testLink(address.text);
                set(() => tested = peer == null ? null : 'Answered: …${peer.substring(peer.length > 8 ? peer.length - 8 : 0)}');
              },
              child: const Text('Test'),
            ),
          FilledButton(
            key: const Key('link-edit-save'),
            onPressed: () async {
              var at = l.address;
              if (!l.fixed && address.text.trim() != l.address) {
                if (!await c.editLink(l.address, address.text)) {
                  set(() {});
                  return;
                }
                at = address.text.trim();
              }
              if (name.text != l.name) await c.renameLink(at, name.text);
              if (ctx.mounted && c.notice == null) Navigator.of(ctx).pop();
            },
            child: const Text('Save'),
          ),
        ],
      ),
    ),
  );
}

Future<void> confirmDeleteLink(BuildContext context, BragiController c, LinkView l) async {
  final yes = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: Text('Delete the link ${l.name}?'),
      content: const Text(
        'Bragi forgets this address and stops dialing it. No session is bound to it (sessions use the whole pool) and no conversation is deleted.',
      ),
      actions: [
        TextButton(onPressed: () => Navigator.of(ctx).pop(false), child: const Text('Cancel')),
        FilledButton(key: const Key('delete-link-ok'), onPressed: () => Navigator.of(ctx).pop(true), child: const Text('Delete link')),
      ],
    ),
  );
  if (yes == true) await c.deleteLink(l.address);
}
