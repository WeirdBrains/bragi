// Settings (LF-11): OpenAI-compatible endpoints and Weirdnet links (add, edit, delete, test,
// reorder), and history. Every change is one core call through the controller; the dialogs only
// ask first. Deleting an endpoint asks where its sessions go; no history is ever deleted by it.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'delete_ui.dart';
import 'models_ui.dart';
import 'on_device_ui.dart';
import 'theme.dart';
import 'wake_ui.dart';

Widget _heading(String text) => Padding(
  padding: const EdgeInsets.fromLTRB(16, 20, 16, 4),
  child: Text(
    text,
    style: TextStyle(fontSize: 11, color: BragiColors.textSecondary),
  ),
);

/// Settings, Video: "Play clips on all my devices" (`video.store_clip`, off by default). On, a
/// clip attached here (up to the cap) goes into its message, so linked devices can play it.
class StoreClipsTile extends StatefulWidget {
  const StoreClipsTile({super.key, required this.api});

  final BragiApi api;

  @override
  State<StoreClipsTile> createState() => _StoreClipsTileState();
}

class _StoreClipsTileState extends State<StoreClipsTile> {
  late bool _on = widget.api.videoStoreClip();
  String? _error;

  void _set(bool on) {
    try {
      final now = widget.api.setVideoStoreClip(on);
      setState(() {
        _on = now;
        _error = null;
      });
    } catch (e) {
      setState(() => _error = '$e');
    }
  }

  @override
  Widget build(BuildContext context) {
    final mib = widget.api.videoStoredClipMaxMib();
    return SwitchListTile(
      key: const Key('store-clips'),
      dense: true,
      title: const Text('Play clips on all my devices'),
      subtitle: Text(
        _error ??
            'Copies each clip you attach (up to $mib MiB) into its message, so your linked devices can play it. It syncs and is deleted with the message. Larger clips play only here.',
        style: TextStyle(
          fontSize: 12,
          color: _error == null ? BragiColors.textSecondary : BragiColors.error,
        ),
      ),
      value: _on,
      onChanged: _set,
    );
  }
}

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final c = controller;
        return Scaffold(
          appBar: AppBar(title: const Text('Settings')),
          body: SingleChildScrollView(
            key: const Key('settings-list'),
            // Keep every section built: the page is short, and tests (and a phone) would
            // otherwise miss tiles such as Clear all history below the fold.
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _heading('YMIR POOLS'),
                Padding(
                  padding: EdgeInsets.fromLTRB(16, 0, 16, 8),
                  child: Text(
                    'This device can be linked to more than one Ymir cloud. Switch which one chats go to; each pool keeps its own history.',
                    style: TextStyle(
                      fontSize: 12,
                      color: BragiColors.textSecondary,
                    ),
                  ),
                ),
                if (c.pools.isEmpty)
                  ListTile(
                    key: Key('no-pools'),
                    dense: true,
                    title: Text(
                      'No Ymir pools yet.',
                      style: TextStyle(color: BragiColors.textSecondary),
                    ),
                  ),
                for (var i = 0; i < c.pools.length; i++)
                  _PoolTile(controller: c, index: i),
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: TextButton.icon(
                      key: const Key('add-pool'),
                      icon: const Icon(Icons.add),
                      label: const Text('Add a pool'),
                      onPressed: () async {
                        Navigator.of(context).pop();
                        await c.addPool();
                      },
                    ),
                  ),
                ),
                _heading('OPENAI-COMPATIBLE ENDPOINTS'),
                Padding(
                  padding: EdgeInsets.fromLTRB(16, 0, 16, 8),
                  child: Text(
                    'Any server that speaks the OpenAI chat API. "Fallback" endpoints answer only when the Weirdnet pool can’t, in this order; "Alongside" ones can be picked for a chat next to the pool. API keys are stored sealed on this device and are never synced.',
                    style: TextStyle(
                      fontSize: 12,
                      color: BragiColors.textSecondary,
                    ),
                  ),
                ),
                if (c.endpoints.isEmpty)
                  ListTile(
                    key: Key('no-endpoints'),
                    dense: true,
                    title: Text(
                      'No endpoints yet.',
                      style: TextStyle(color: BragiColors.textSecondary),
                    ),
                  ),
                for (var i = 0; i < c.endpoints.length; i++)
                  _EndpointTile(controller: c, index: i),
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: TextButton.icon(
                      key: const Key('add-endpoint'),
                      icon: const Icon(Icons.add),
                      label: const Text('Add endpoint'),
                      onPressed: () => showEndpointDialog(context, c),
                    ),
                  ),
                ),
                _heading('WEIRDNET LINKS'),
                Padding(
                  padding: EdgeInsets.fromLTRB(16, 0, 16, 8),
                  child: Text(
                    'Node addresses this device dials (from `ymirctl pool addresses`).',
                    style: TextStyle(
                      fontSize: 12,
                      color: BragiColors.textSecondary,
                    ),
                  ),
                ),
                if (c.links.isEmpty)
                  ListTile(
                    key: Key('no-links'),
                    dense: true,
                    title: Text(
                      'No links yet.',
                      style: TextStyle(color: BragiColors.textSecondary),
                    ),
                  ),
                for (var i = 0; i < c.links.length; i++)
                  _LinkTile(controller: c, index: i),
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: TextButton.icon(
                      key: const Key('add-link'),
                      icon: const Icon(Icons.add),
                      label: const Text('Add link'),
                      onPressed: () => showLinkDialog(context, c, null),
                    ),
                  ),
                ),
                _heading('MODELS'),
                ListTile(
                  key: const Key('open-models'),
                  dense: true,
                  leading: const Icon(Icons.memory_outlined),
                  title: const Text('Models on your pool'),
                  subtitle: const Text(
                    'Search, pull, list and remove models on your Ymir nodes.',
                  ),
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => ModelsScreen(controller: c),
                    ),
                  ),
                ),
                _heading('VIDEO'),
                StoreClipsTile(api: c.api),
                _heading('WAKE-UPS'),
                WakeSection(api: c.api),
                _heading('ON-DEVICE MODEL'),
                OnDeviceSection(api: c.api, onChanged: c.refreshTargets),
                _heading('HISTORY'),
                ClearHistoryTile(controller: controller),
                if (c.notice != null)
                  Padding(
                    padding: const EdgeInsets.all(16),
                    child: Text(
                      c.notice!,
                      key: const Key('settings-notice'),
                      style: TextStyle(
                        color: c.notice!.startsWith('Saved ')
                            ? BragiColors.warm
                            : BragiColors.error,
                      ),
                    ),
                  ),
                const SizedBox(height: 48),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _EndpointTile extends StatelessWidget {
  const _EndpointTile({required this.controller, required this.index});

  final BragiController controller;
  final int index;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final e = c.endpoints[index];
    final last = index == c.endpoints.length - 1;
    final mode = e.mode == 'fallback'
        ? 'fallback #${c.endpoints.take(index + 1).where((x) => x.mode == 'fallback').length}'
        : 'alongside the pool';
    return ListTile(
      key: Key('endpoint-${e.id}'),
      dense: true,
      title: Text(e.name),
      subtitle: Text(
        '${e.baseUrl}\n$mode${e.enabled ? '' : ' · off'}${e.model == null ? '' : ' · ${e.model}'}${e.hasKey ? ' · key set' : ''}${e.insecure ? ' · not encrypted (http)' : ''}',
      ),
      isThreeLine: true,
      trailing: Wrap(
        children: [
          IconButton(
            key: Key('endpoint-up-${e.id}'),
            tooltip: 'Move up',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.arrow_upward, size: 18),
            onPressed: index == 0
                ? null
                : () => c.moveEndpoint(e.id, index - 1),
          ),
          IconButton(
            key: Key('endpoint-down-${e.id}'),
            tooltip: 'Move down',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.arrow_downward, size: 18),
            onPressed: last ? null : () => c.moveEndpoint(e.id, index + 1),
          ),
          IconButton(
            key: Key('endpoint-test-${e.id}'),
            tooltip: 'Test',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.network_check, size: 18),
            onPressed: () async {
              final r = await c.testEndpoint(e.id);
              if (r == null || !context.mounted) return;
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(
                  key: const Key('endpoint-test-result'),
                  content: Text(
                    '${e.name}: ${r.message}${r.ok ? ' (${r.latencyMs} ms)' : ''}',
                  ),
                ),
              );
            },
          ),
          IconButton(
            key: Key('endpoint-edit-${e.id}'),
            tooltip: 'Edit',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.edit_outlined, size: 18),
            onPressed: () => showEndpointDialog(context, c, existing: e),
          ),
          IconButton(
            key: Key('endpoint-delete-${e.id}'),
            tooltip: 'Delete',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.delete_outline, size: 18),
            onPressed: () => confirmDeleteEndpoint(context, c, e),
          ),
        ],
      ),
    );
  }
}

/// The add / edit form. The key field is empty when editing; leaving it empty keeps the key,
/// typing replaces it, and "Remove key" clears it.
Future<void> showEndpointDialog(
  BuildContext context,
  BragiController c, {
  EndpointView? existing,
}) {
  return showDialog<void>(
    context: context,
    builder: (_) => _EndpointDialog(controller: c, existing: existing),
  );
}

class _EndpointDialog extends StatefulWidget {
  const _EndpointDialog({required this.controller, this.existing});

  final BragiController controller;
  final EndpointView? existing;

  @override
  State<_EndpointDialog> createState() => _EndpointDialogState();
}

class _EndpointDialogState extends State<_EndpointDialog> {
  late final _name = TextEditingController(text: widget.existing?.name ?? '');
  late final _url = TextEditingController(text: widget.existing?.baseUrl ?? '');
  final _key = TextEditingController();
  late final _model = TextEditingController(text: widget.existing?.model ?? '');
  late bool _alongside = (widget.existing?.mode ?? 'fallback') == 'alongside';
  late bool _enabled = widget.existing?.enabled ?? true;
  bool _clearKey = false;
  bool _obscure = true;
  String? _result;
  bool _resultOk = false;
  String? _error;

  @override
  void dispose() {
    _name.dispose();
    _url.dispose();
    _key.dispose();
    _model.dispose();
    super.dispose();
  }

  EndpointForm _form() {
    final key = _key.text.trim();
    return EndpointForm(
      name: _name.text,
      baseUrl: _url.text,
      model: _model.text.trim().isEmpty ? null : _model.text.trim(),
      mode: _alongside ? 'alongside' : 'fallback',
      enabled: _enabled,
      apiKey: key.isEmpty ? null : key,
      keyChange: key.isNotEmpty ? 'set' : (_clearKey ? 'clear' : 'keep'),
    );
  }

  Future<void> _test() async {
    final key = _key.text.trim();
    final id = widget.existing?.id;
    // A saved endpoint with its own key is tested as saved when no new key was typed.
    final r =
        (id != null &&
            key.isEmpty &&
            !_clearKey &&
            _url.text.trim() == widget.existing!.baseUrl)
        ? await widget.controller.testEndpoint(id)
        : await widget.controller.testAddress(
            _url.text,
            key.isEmpty ? null : key,
          );
    if (!mounted) return;
    setState(() {
      _result = r == null
          ? widget.controller.notice
          : '${r.message}${r.ok ? ' (${r.latencyMs} ms)' : ''}';
      _resultOk = r?.ok ?? false;
      if (r != null &&
          r.ok &&
          _model.text.trim().isEmpty &&
          r.models.isNotEmpty) {
        _model.text = r.models.first;
      }
    });
  }

  Future<void> _save() async {
    final ok = await widget.controller.saveEndpoint(
      widget.existing?.id,
      _form(),
    );
    if (!mounted) return;
    if (ok) {
      Navigator.of(context).pop();
    } else {
      setState(() => _error = widget.controller.notice);
    }
  }

  @override
  Widget build(BuildContext context) {
    final editing = widget.existing != null;
    return AlertDialog(
      title: Text(editing ? 'Edit endpoint' : 'Add endpoint'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              TextField(
                key: const Key('ep-name'),
                controller: _name,
                decoration: const InputDecoration(labelText: 'Name'),
              ),
              TextField(
                key: const Key('ep-url'),
                controller: _url,
                keyboardType: TextInputType.url,
                decoration: const InputDecoration(
                  labelText: 'Address',
                  hintText: 'https://api.example.com/v1',
                ),
              ),
              TextField(
                key: const Key('ep-key'),
                controller: _key,
                obscureText: _obscure,
                enableSuggestions: false,
                autocorrect: false,
                decoration: InputDecoration(
                  labelText: editing && widget.existing!.hasKey
                      ? 'API key (leave empty to keep it)'
                      : 'API key (optional)',
                  suffixIcon: IconButton(
                    icon: Icon(
                      _obscure
                          ? Icons.visibility_outlined
                          : Icons.visibility_off_outlined,
                    ),
                    onPressed: () => setState(() => _obscure = !_obscure),
                  ),
                ),
              ),
              if (editing && widget.existing!.hasKey)
                CheckboxListTile(
                  key: const Key('ep-clear-key'),
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  value: _clearKey,
                  onChanged: (v) => setState(() => _clearKey = v ?? false),
                  title: const Text('Remove the saved key'),
                ),
              TextField(
                key: const Key('ep-model'),
                controller: _model,
                decoration: const InputDecoration(
                  labelText: 'Model (optional; Test fills it in)',
                ),
              ),
              SwitchListTile(
                key: const Key('ep-alongside'),
                dense: true,
                contentPadding: EdgeInsets.zero,
                value: _alongside,
                onChanged: (v) => setState(() => _alongside = v),
                title: const Text('Use alongside the pool'),
                subtitle: Text(
                  _alongside
                      ? 'Can be picked for a chat.'
                      : 'Fallback only: used when the pool can’t answer.',
                ),
              ),
              SwitchListTile(
                key: const Key('ep-enabled'),
                dense: true,
                contentPadding: EdgeInsets.zero,
                value: _enabled,
                onChanged: (v) => setState(() => _enabled = v),
                title: const Text('On'),
              ),
              if (_result != null)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    _result!,
                    key: const Key('ep-test-result'),
                    style: TextStyle(
                      color: _resultOk ? BragiColors.warm : BragiColors.error,
                    ),
                  ),
                ),
              if (_error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    _error!,
                    key: const Key('ep-error'),
                    style: TextStyle(color: BragiColors.error),
                  ),
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          key: const Key('ep-test'),
          onPressed: _test,
          child: const Text('Test'),
        ),
        TextButton(
          key: const Key('ep-cancel'),
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('ep-save'),
          onPressed: _save,
          child: Text(editing ? 'Save' : 'Add'),
        ),
      ],
    );
  }
}

/// Asks before deleting an endpoint. With sessions bound to it, also asks where they go: another
/// target, or unbound (they ask for a target on the next message). Conversations are never deleted.
Future<void> confirmDeleteEndpoint(
  BuildContext context,
  BragiController c,
  EndpointView e,
) async {
  final others = c.targets.where((t) => t.endpoint != e.id).toList();
  final choice = await showDialog<_Rebind>(
    context: context,
    builder: (_) => _DeleteEndpointDialog(endpoint: e, targets: others),
  );
  if (choice == null || !context.mounted) return;
  final r = await c.deleteEndpoint(e.id, choice.target);
  if (r == null || !context.mounted) return;
  final msg = r.rebound > 0
      ? 'Deleted ${e.name}; ${r.rebound} chat${r.rebound == 1 ? '' : 's'} moved to ${choice.target!.label}.'
      : r.unbound > 0
      ? 'Deleted ${e.name}; ${r.unbound} chat${r.unbound == 1 ? '' : 's'} need a new target. History is kept.'
      : 'Deleted ${e.name}.';
  ScaffoldMessenger.of(context).showSnackBar(
    SnackBar(key: const Key('endpoint-deleted'), content: Text(msg)),
  );
}

class _Rebind {
  const _Rebind(this.target);
  final TargetView? target;
}

class _DeleteEndpointDialog extends StatefulWidget {
  const _DeleteEndpointDialog({required this.endpoint, required this.targets});

  final EndpointView endpoint;
  final List<TargetView> targets;

  @override
  State<_DeleteEndpointDialog> createState() => _DeleteEndpointDialogState();
}

class _DeleteEndpointDialogState extends State<_DeleteEndpointDialog> {
  /// -1: leave unbound; otherwise an index into the targets.
  int _pick = 0;

  @override
  Widget build(BuildContext context) {
    final e = widget.endpoint;
    final n = e.sessions;
    final canMove = widget.targets.isNotEmpty;
    if (!canMove) _pick = -1;
    return AlertDialog(
      title: Text('Delete ${e.name}?'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'Its address and API key are removed from this device. Your chats and their history are kept.',
            key: Key('ep-delete-text'),
          ),
          if (n > 0) ...[
            const SizedBox(height: 12),
            Text(
              '$n chat${n == 1 ? ' is' : 's are'} bound to it. Where should ${n == 1 ? 'it' : 'they'} go?',
              key: const Key('ep-delete-bound'),
            ),
            RadioGroup<int>(
              groupValue: _pick,
              onChanged: (v) => setState(() => _pick = v ?? -1),
              child: Column(
                children: [
                  for (var i = 0; i < widget.targets.length; i++)
                    RadioListTile<int>(
                      key: Key('rebind-$i'),
                      dense: true,
                      value: i,
                      title: Text('Move to ${widget.targets[i].label}'),
                    ),
                  const RadioListTile<int>(
                    key: Key('rebind-unbound'),
                    dense: true,
                    value: -1,
                    title: Text('Leave without a target'),
                  ),
                ],
              ),
            ),
          ],
        ],
      ),
      actions: [
        TextButton(
          key: const Key('ep-delete-cancel'),
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('ep-delete-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: () => Navigator.of(context)
              .pop(_Rebind(n > 0 && _pick >= 0 ? widget.targets[_pick] : null)),
          child: const Text('Delete'),
        ),
      ],
    );
  }
}

class _LinkTile extends StatelessWidget {
  const _LinkTile({required this.controller, required this.index});

  final BragiController controller;
  final int index;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final l = c.links[index];
    return ListTile(
      key: Key('link-$index'),
      dense: true,
      leading: Icon(
        Icons.circle,
        size: 10,
        color: l.online ? BragiColors.warm : BragiColors.textSecondary,
      ),
      title: Text(l.address, overflow: TextOverflow.ellipsis),
      subtitle: Text(l.online ? 'answering' : 'not answering'),
      trailing: Wrap(
        children: [
          IconButton(
            key: Key('link-edit-$index'),
            tooltip: 'Edit',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.edit_outlined, size: 18),
            onPressed: () => showLinkDialog(context, c, l.address),
          ),
          IconButton(
            key: Key('link-delete-$index'),
            tooltip: 'Delete',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.delete_outline, size: 18),
            onPressed: () => confirmDeleteLink(context, c, l.address),
          ),
        ],
      ),
    );
  }
}

/// Adds a link (`old` null) or changes one's address.
Future<void> showLinkDialog(
  BuildContext context,
  BragiController c,
  String? old,
) {
  return showDialog<void>(
    context: context,
    builder: (_) => _LinkDialog(controller: c, old: old),
  );
}

class _LinkDialog extends StatefulWidget {
  const _LinkDialog({required this.controller, required this.old});

  final BragiController controller;
  final String? old;

  @override
  State<_LinkDialog> createState() => _LinkDialogState();
}

class _LinkDialogState extends State<_LinkDialog> {
  late final _address = TextEditingController(text: widget.old ?? '');
  String? _error;

  @override
  void dispose() {
    _address.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final a = _address.text.trim();
    final ok = widget.old == null
        ? await widget.controller.linkDevice(a)
        : await widget.controller.editLink(widget.old!, a);
    if (!mounted) return;
    if (ok) {
      if (mounted) Navigator.of(context).pop();
      await widget.controller.reloadSettings();
    } else {
      setState(
        () => _error =
            widget.controller.notice ??
            'Paste a node address that starts with /ip4/ and ends with /p2p/…',
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.old == null ? 'Add link' : 'Edit link'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            key: const Key('link-address'),
            controller: _address,
            decoration: const InputDecoration(
              labelText: 'Node address',
              hintText: '/ip4/…/tcp/…/p2p/…',
            ),
          ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                _error!,
                key: const Key('link-error'),
                style: TextStyle(color: BragiColors.error),
              ),
            ),
        ],
      ),
      actions: [
        TextButton(
          key: const Key('link-cancel'),
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('link-save'),
          onPressed: _save,
          child: Text(widget.old == null ? 'Add' : 'Save'),
        ),
      ],
    );
  }
}

Future<void> confirmDeleteLink(
  BuildContext context,
  BragiController c,
  String address,
) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (d) => AlertDialog(
      title: const Text('Delete this link?'),
      content: Text(
        'Bragi stops dialing $address. Your chats and history are kept.',
      ),
      actions: [
        TextButton(
          key: const Key('link-delete-cancel'),
          onPressed: () => Navigator.of(d).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('link-delete-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: () => Navigator.of(d).pop(true),
          child: const Text('Delete'),
        ),
      ],
    ),
  );
  if (ok == true) await c.removeLink(address);
}

class _PoolTile extends StatelessWidget {
  const _PoolTile({required this.controller, required this.index});

  final BragiController controller;
  final int index;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final p = c.pools[index];
    final status = p.active ? 'Active' : (p.linked ? 'Saved' : 'Not linked');
    return ListTile(
      key: Key('pool-${p.id}'),
      dense: true,
      title: Text(p.name, overflow: TextOverflow.ellipsis),
      subtitle: Text(status),
      trailing: Wrap(
        children: [
          if (!p.active)
            TextButton(
              key: Key('pool-switch-${p.id}'),
              onPressed: () => c.switchPool(p.id),
              child: const Text('Switch'),
            ),
          IconButton(
            key: Key('pool-rename-${p.id}'),
            tooltip: 'Rename',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.edit_outlined, size: 18),
            onPressed: () => showRenamePoolDialog(context, c, p.id, p.name),
          ),
          IconButton(
            key: Key('pool-delete-${p.id}'),
            tooltip: 'Forget',
            visualDensity: VisualDensity.compact,
            icon: const Icon(Icons.delete_outline, size: 18),
            onPressed: () => confirmDeletePool(context, c, p.id, p.name),
          ),
        ],
      ),
    );
  }
}

Future<void> showRenamePoolDialog(
  BuildContext context,
  BragiController c,
  String id,
  String name,
) {
  return showDialog<void>(
    context: context,
    builder: (_) => _RenamePoolDialog(controller: c, id: id, name: name),
  );
}

class _RenamePoolDialog extends StatefulWidget {
  const _RenamePoolDialog({
    required this.controller,
    required this.id,
    required this.name,
  });

  final BragiController controller;
  final String id;
  final String name;

  @override
  State<_RenamePoolDialog> createState() => _RenamePoolDialogState();
}

class _RenamePoolDialogState extends State<_RenamePoolDialog> {
  late final _name = TextEditingController(text: widget.name);

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    await widget.controller.renamePool(widget.id, _name.text.trim());
    if (mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Rename pool'),
      content: TextField(
        key: const Key('pool-name'),
        controller: _name,
        decoration: const InputDecoration(labelText: 'Name'),
      ),
      actions: [
        TextButton(
          key: const Key('pool-rename-cancel'),
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('pool-rename-save'),
          onPressed: _save,
          child: const Text('Save'),
        ),
      ],
    );
  }
}

Future<void> confirmDeletePool(
  BuildContext context,
  BragiController c,
  String id,
  String name,
) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (d) => AlertDialog(
      title: Text('Forget $name?'),
      content: const Text(
        'This device forgets that Ymir pool. Its identity and history on this device are deleted. Other devices are not changed.',
      ),
      actions: [
        TextButton(
          key: const Key('pool-delete-cancel'),
          onPressed: () => Navigator.of(d).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('pool-delete-confirm'),
          style: FilledButton.styleFrom(backgroundColor: BragiColors.error),
          onPressed: () => Navigator.of(d).pop(true),
          child: const Text('Forget'),
        ),
      ],
    ),
  );
  if (ok == true) await c.removePool(id);
}
