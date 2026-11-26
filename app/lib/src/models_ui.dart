// Models on the user's Ymir pool (LF-30): search the Hugging Face Hub with a fit badge per node,
// pull to a node, the best one (auto) or all of them with progress and cancel, see what each node
// has, and remove with a confirm. Every action is one core call; this only asks and shows. The
// nodes admit only devices linked to the owner; one that refuses this device shows "needs owner
// device" and its controls are off.
import 'dart:async';

import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'theme.dart';
import 'training_view_ui.dart';
import 'upload_ui.dart';

/// `4.4 GB`, like the core's `human_bytes`.
String humanBytes(num n) {
  const units = ['B', 'kB', 'MB', 'GB', 'TB'];
  var v = n.toDouble();
  var i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return i == 0 ? '${n.toInt()} B' : '${v.toStringAsFixed(1)} ${units[i]}';
}

String _count(int n) => n >= 1000000 ? '${(n / 1000000).toStringAsFixed(1)}M' : n >= 1000 ? '${(n / 1000).toStringAsFixed(1)}k' : '$n';

String statusLabel(String status) => switch (status) {
  'ready' => 'Ready',
  'not_model_node' => 'Not a model node',
  'needs_owner_device' => 'Needs owner device',
  _ => 'Unreachable',
};

Color statusColor(String status) => switch (status) {
  'ready' => BragiColors.warm,
  'not_model_node' => BragiColors.textSecondary,
  _ => BragiColors.busy,
};

/// A pull as the screen shows it: one per node, then the outcome.
class _PullRun {
  _PullRun(this.repo, this.runId);

  final String repo;
  final String runId;
  String? chosen;
  final Map<String, ({String label, int done, int total})> progress = {};
  final Map<String, String> results = {};
  bool finished = false;
  String? ended;
}

class ModelsScreen extends StatefulWidget {
  const ModelsScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  State<ModelsScreen> createState() => _ModelsScreenState();
}

class _ModelsScreenState extends State<ModelsScreen> {
  final _query = TextEditingController();
  List<NodeModelsView>? _nodes;
  String? _error;
  List<HubHitView> _hits = const [];
  final Map<String, Map<String, NodeFitView>> _fits = {};
  bool _searching = false;
  String? _searchError;
  String? _searchRun;
  String _target = 'auto';
  bool _serve = true;
  final Map<String, String?> _quant = {};
  _PullRun? _pull;
  List<PendingUploadView> _pending = const [];
  StreamSubscription<ModelsEventView>? _searchSub;
  StreamSubscription<ModelsEventView>? _pullSub;

  BragiApi get _api => widget.controller.api;

  @override
  void initState() {
    super.initState();
    _reload();
  }

  @override
  void dispose() {
    _searchSub?.cancel();
    _pullSub?.cancel();
    if (_searchRun != null && _searching) _api.cancel(_searchRun!);
    _query.dispose();
    super.dispose();
  }

  Future<void> _reload() async {
    try {
      final nodes = await _api.modelsOverview();
      List<PendingUploadView> pending;
      try {
        pending = await _api.modelsPendingUploads();
      } catch (_) {
        pending = const [];
      }
      if (!mounted) return;
      setState(() {
        _nodes = nodes;
        _pending = pending;
        _error = null;
        // A node that went away cannot stay the target.
        if (_target != 'auto' && _target != 'all' && !nodes.any((n) => n.node.peer == _target && n.status == 'ready')) _target = 'auto';
      });
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  List<NodeModelsView> get _ready => (_nodes ?? const []).where((n) => n.status == 'ready').toList();

  /// Every node refused this device: the controls stay off and the banner says what is needed.
  bool get _ownerBlocked => (_nodes ?? const []).isNotEmpty && (_nodes ?? const []).every((n) => n.status == 'needs_owner_device');

  Future<void> _search() async {
    final q = _query.text.trim();
    if (q.isEmpty) return;
    await _searchSub?.cancel();
    setState(() {
      _searching = true;
      _searchError = null;
      _hits = const [];
      _fits.clear();
    });
    try {
      final run = await _api.modelsSearch(q);
      _searchRun = run;
      _searchSub = _api.followModels(run).listen((e) {
        if (!mounted) return;
        setState(() {
          switch (e.kind) {
            case 'hits':
              _hits = e.hits;
            case 'fit':
              _fits.putIfAbsent(e.repo!, () => {})[e.fit!.node.peer] = e.fit!;
            case 'search_done':
              _searching = false;
              _searchError = e.text;
            case 'cancelled':
              _searching = false;
            case 'failed':
              _searching = false;
              _searchError = e.text;
          }
        });
      }, onError: (Object e) {
        if (mounted) {
          setState(() {
            _searching = false;
            _searchError = '$e';
          });
        }
      });
    } catch (e) {
      if (mounted) {
        setState(() {
          _searching = false;
          _searchError = '$e';
        });
      }
    }
  }

  Future<void> _startPull(HubHitView hit) async {
    if (_pull != null && !_pull!.finished) return;
    try {
      final run = await _api.modelsPull(hit.repo, _quant[hit.repo], _target, _serve);
      final pull = _PullRun(hit.repo, run);
      setState(() => _pull = pull);
      _pullSub = _api.followModels(run).listen((e) {
        if (!mounted) return;
        setState(() {
          switch (e.kind) {
            case 'chosen':
              pull.chosen = '${e.node!.label}: ${e.text}';
            case 'progress':
              pull.progress[e.node!.peer] = (label: e.node!.label, done: e.done.toInt(), total: e.total.toInt());
            case 'node_done':
              pull.results[e.node!.peer] = e.text != null
                  ? e.text!
                  : '${e.node!.label}: ${e.model} (${humanBytes(e.size.toInt())}) ${e.served ? 'downloaded and serving' : 'downloaded'}';
              final p = pull.progress[e.node!.peer];
              if (p != null && e.text == null) pull.progress[e.node!.peer] = (label: p.label, done: p.total, total: p.total);
            case 'done':
              pull.finished = true;
              pull.ended = 'Done';
              _reload();
            case 'cancelled':
              pull.finished = true;
              pull.ended = 'Cancelled. What was downloaded is kept; pull again to resume.';
              _reload();
            case 'failed':
              pull.finished = true;
              pull.ended = e.text;
          }
        });
      });
    } catch (e) {
      if (mounted) setState(() => _searchError = '$e');
    }
  }

  void _cancelPull() {
    final p = _pull;
    if (p != null && !p.finished) _api.cancel(p.runId);
  }

  Future<void> _remove(NodeModelsView node, StoredModelView m) async {
    final yes = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('remove-dialog'),
        title: Text('Remove ${m.id}?'),
        content: Text(
          'Deletes ${humanBytes(m.size.toInt())} from ${node.node.label}${m.servedAs == null ? '' : ' and stops serving it'}. You can pull it again later.',
        ),
        actions: [
          TextButton(key: const Key('remove-cancel'), onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          TextButton(
            key: const Key('remove-confirm'),
            onPressed: () => Navigator.pop(context, true),
            child: Text('Remove', style: TextStyle(color: BragiColors.error)),
          ),
        ],
      ),
    );
    if (yes != true) return;
    try {
      final freed = await _api.modelsRemove(node.node.peer, m.id, true);
      if (mounted) setState(() => _searchError = 'Removed ${m.id} from ${node.node.label}: ${humanBytes(freed.toInt())} freed.');
    } catch (e) {
      if (mounted) setState(() => _searchError = '$e');
    }
    await _reload();
  }

  Future<void> _openUpload([UploadSpecView? resume]) async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(builder: (_) => UploadScreen(api: _api, nodes: _nodes ?? const [], resume: resume)),
    );
    await _reload();
  }

  Future<void> _forgetUpload(PendingUploadView p) async {
    try {
      await _api.modelsForgetUpload(p.spec.path, p.spec.peer);
    } catch (e) {
      if (mounted) setState(() => _searchError = '$e');
    }
    await _reload();
  }

  Widget _uploadRow() => Padding(
    padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
    child: Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        key: const Key('models-upload'),
        icon: const Icon(Icons.upload_file, size: 18),
        label: const Text('Upload a model'),
        onPressed: _ready.isEmpty ? null : _openUpload,
      ),
    ),
  );

  Widget _pendingTile(PendingUploadView p) => ListTile(
    key: Key('pending-${p.spec.peer}-${p.spec.path}'),
    dense: true,
    title: Text(p.label, style: const TextStyle(fontSize: 13)),
    subtitle: Text(humanBytes(p.size.toInt()), style: TextStyle(fontSize: 12, color: BragiColors.textSecondary)),
    trailing: Wrap(
      children: [
        TextButton(key: const Key('pending-resume'), onPressed: () => _openUpload(p.spec), child: const Text('Resume')),
        TextButton(key: const Key('pending-forget'), onPressed: () => _forgetUpload(p), child: const Text('Forget')),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    final nodes = _nodes;
    return Scaffold(
      appBar: AppBar(title: const Text('Models'), actions: [
          IconButton(
            key: const Key('models-training'),
            tooltip: 'Training',
            icon: const Icon(Icons.model_training),
            onPressed: () => Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => TrainingScreen(controller: widget.controller))),
          ),
          IconButton(key: const Key('models-refresh'), tooltip: 'Refresh', icon: const Icon(Icons.refresh), onPressed: _reload),
        ],
      ),
      body: ListView(
        key: const Key('models-list'),
        padding: const EdgeInsets.only(bottom: 24),
        children: [
          if (_error != null) Padding(padding: const EdgeInsets.all(16), child: Text(_error!, style: TextStyle(color: BragiColors.error))),
          if (_ownerBlocked) _ownerBanner(),
          _heading('YOUR NODES'),
          if (nodes == null)
            Padding(padding: EdgeInsets.all(16), child: Text('Asking your devices…', style: TextStyle(color: BragiColors.textSecondary)))
          else if (nodes.isEmpty)
            ListTile(
              key: Key('models-no-nodes'),
              dense: true,
              title: Text('No other devices on your roster yet. Link a Ymir node first.', style: TextStyle(color: BragiColors.textSecondary)),
            ),
          for (final n in nodes ?? const <NodeModelsView>[]) _nodeTile(n),
          if (nodes != null && nodes.isNotEmpty) _uploadRow(),
          if (_pending.isNotEmpty) _heading('UNFINISHED UPLOADS'),
          for (final p in _pending) _pendingTile(p),
          _heading('FIND A MODEL'),
          _searchBar(),
          if (_searchError != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
              child: Text(_searchError!, key: const Key('models-notice'), style: TextStyle(fontSize: 12, color: BragiColors.textSecondary)),
            ),
          if (_searching) const Padding(padding: EdgeInsets.all(16), child: LinearProgressIndicator(key: Key('models-searching'))),
          if (_pull != null) _pullCard(_pull!),
          for (final h in _hits) _hitCard(h),
        ],
      ),
    );
  }

  Widget _heading(String text) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 20, 16, 4),
    child: Text(text, style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
  );

  Widget _ownerBanner() => Container(
    key: const Key('models-owner-banner'),
    margin: const EdgeInsets.all(16),
    padding: const EdgeInsets.all(12),
    decoration: BoxDecoration(color: BragiColors.surface, borderRadius: BorderRadius.circular(8), border: Border.all(color: BragiColors.busy)),
    child: const Text(
      'Your nodes only let devices linked to their owner manage models, and none accepted this device. Link this device from a node you own (Link a device), then come back. Nothing here can change until then.',
      style: TextStyle(fontSize: 13),
    ),
  );

  Widget _nodeTile(NodeModelsView n) {
    final ready = n.status == 'ready';
    return Card(
      key: Key('node-${n.node.peer}'),
      color: BragiColors.surface,
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Expanded(child: Text(n.node.label, style: const TextStyle(fontSize: 15))),
                Text(statusLabel(n.status), key: Key('status-${n.node.peer}'), style: TextStyle(fontSize: 12, color: statusColor(n.status))),
              ],
            ),
            if (n.detail != null && !ready) Text(n.detail!, style: TextStyle(fontSize: 12, color: BragiColors.textSecondary)),
            if (ready && n.models.isEmpty) Text('No models downloaded.', style: TextStyle(fontSize: 12, color: BragiColors.textSecondary)),
            for (final m in n.models)
              Row(
                children: [
                  Expanded(
                    child: Text(
                      '${m.id} · ${humanBytes(m.size.toInt())}${m.complete ? '' : ' · incomplete'}${m.servedAs == null ? '' : ' · serving${m.state == null ? '' : ' (${m.state})'}'}',
                      style: const TextStyle(fontSize: 13),
                    ),
                  ),
                  IconButton(
                    key: Key('remove-${n.node.peer}-${m.id}'),
                    tooltip: 'Remove',
                    visualDensity: VisualDensity.compact,
                    icon: const Icon(Icons.delete_outline, size: 20),
                    onPressed: () => _remove(n, m),
                  ),
                ],
              ),
          ],
        ),
      ),
    );
  }

  Widget _searchBar() {
    final ready = _ready;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: TextField(
                  key: const Key('models-search-field'),
                  controller: _query,
                  enabled: !_ownerBlocked,
                  textInputAction: TextInputAction.search,
                  decoration: const InputDecoration(hintText: 'Search Hugging Face (GGUF), such as deepseek or qwen', isDense: true),
                  onSubmitted: (_) => _search(),
                ),
              ),
              const SizedBox(width: 8),
              FilledButton(key: const Key('models-search'), onPressed: _ownerBlocked || _searching ? null : _search, child: const Text('Search')),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              const Text('Pull to ', style: TextStyle(fontSize: 13)),
              DropdownButton<String>(
                key: const Key('models-target'),
                value: _target,
                items: [
                  const DropdownMenuItem(value: 'auto', child: Text('Auto (best fit)')),
                  const DropdownMenuItem(value: 'all', child: Text('All nodes')),
                  for (final n in ready) DropdownMenuItem(value: n.node.peer, child: Text(n.node.label)),
                ],
                onChanged: _ownerBlocked ? null : (v) => setState(() => _target = v ?? 'auto'),
              ),
              const Spacer(),
              const Text('Serve now', style: TextStyle(fontSize: 12)),
              Switch(key: const Key('models-serve'), value: _serve, onChanged: (v) => setState(() => _serve = v)),
            ],
          ),
        ],
      ),
    );
  }

  Widget _hitCard(HubHitView h) {
    final fits = _fits[h.repo] ?? const <String, NodeFitView>{};
    final quants = <String>{for (final f in fits.values) ...f.quants.map((q) => q.id)}.toList();
    final busy = _pull != null && !_pull!.finished;
    return Card(
      key: Key('hit-${h.repo}'),
      color: BragiColors.surface,
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(h.repo, style: const TextStyle(fontSize: 15)),
            Text(
              '${_count(h.downloads.toInt())} downloads · ${_count(h.likes.toInt())} likes${h.gated ? ' · gated (needs a Hub token on the node)' : ''}',
              style: TextStyle(fontSize: 12, color: BragiColors.textSecondary),
            ),
            const SizedBox(height: 6),
            Wrap(spacing: 6, runSpacing: 4, children: [for (final n in _nodes ?? const <NodeModelsView>[]) _badge(h.repo, n, fits[n.node.peer])]),
            const SizedBox(height: 4),
            Row(
              children: [
                if (quants.isNotEmpty)
                  DropdownButton<String?>(
                    key: Key('quant-${h.repo}'),
                    value: _quant[h.repo],
                    hint: const Text('Best that fits'),
                    items: [
                      const DropdownMenuItem<String?>(value: null, child: Text('Best that fits')),
                      for (final q in quants) DropdownMenuItem<String?>(value: q, child: Text(q)),
                    ],
                    onChanged: (v) => setState(() => _quant[h.repo] = v),
                  ),
                const Spacer(),
                FilledButton(key: Key('pull-${h.repo}'), onPressed: busy || _ownerBlocked ? null : () => _startPull(h), child: const Text('Pull')),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _badge(String repo, NodeModelsView n, NodeFitView? f) {
    var text = '${n.node.label}: checking…';
    var color = BragiColors.textSecondary;
    if (n.status != 'ready') {
      text = '${n.node.label}: ${statusLabel(n.status).toLowerCase()}';
      color = BragiColors.busy;
    } else if (f != null) {
      switch (f.verdict) {
        case 'fits':
          text = '${n.node.label}: fits ${f.best} (${humanBytes(f.bestSize.toInt())})';
          color = BragiColors.warm;
        case 'too_big':
          text = '${n.node.label}: too big for ${humanBytes(f.usableBytes.toInt())}';
          color = BragiColors.error;
        case 'no_budget':
          text = '${n.node.label}: no memory budget; name a quant';
          color = BragiColors.busy;
        default:
          text = '${n.node.label}: unknown';
      }
    }
    return Container(
      key: Key('fit-$repo-${n.node.peer}'),
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
      decoration: BoxDecoration(color: BragiColors.background, borderRadius: BorderRadius.circular(12), border: Border.all(color: color)),
      child: Text(text, style: TextStyle(fontSize: 12, color: color)),
    );
  }

  Widget _pullCard(_PullRun p) {
    return Card(
      key: const Key('pull-card'),
      color: BragiColors.surface,
      margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Expanded(child: Text('Pulling ${p.repo}', style: const TextStyle(fontSize: 15))),
                if (!p.finished) TextButton(key: const Key('pull-cancel'), onPressed: _cancelPull, child: const Text('Cancel')),
                if (p.finished) IconButton(key: const Key('pull-dismiss'), tooltip: 'Dismiss', icon: const Icon(Icons.close, size: 18), onPressed: () => setState(() => _pull = null)),
              ],
            ),
            if (p.chosen != null) Text('Auto picked ${p.chosen}', key: const Key('pull-chosen'), style: TextStyle(fontSize: 12, color: BragiColors.textSecondary)),
            for (final e in p.progress.entries)
              Padding(
                padding: const EdgeInsets.only(top: 6),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '${e.value.label}: ${humanBytes(e.value.done)} of ${humanBytes(e.value.total)}',
                      key: Key('pull-progress-text-${e.key}'),
                      style: const TextStyle(fontSize: 12),
                    ),
                    LinearProgressIndicator(key: Key('pull-progress-${e.key}'), value: e.value.total == 0 ? null : e.value.done / e.value.total),
                  ],
                ),
              ),
            for (final r in p.results.values) Padding(padding: const EdgeInsets.only(top: 4), child: Text(r, style: const TextStyle(fontSize: 12))),
            if (p.ended != null) Padding(padding: const EdgeInsets.only(top: 4), child: Text(p.ended!, key: const Key('pull-ended'), style: TextStyle(fontSize: 12, color: BragiColors.textSecondary))),
            if (p.finished && p.results.isNotEmpty && p.results.values.any((r) => r.contains('serving')))
              Padding(
                padding: EdgeInsets.only(top: 4),
                child: Text('Serving lasts until the node restarts. To keep it, add it to that node’s node.toml.', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
              ),
          ],
        ),
      ),
    );
  }
}
