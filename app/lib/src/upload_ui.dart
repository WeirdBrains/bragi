// Upload a model from this device to one of the user's model nodes, like the desktop dialog: the
// file is read and hashed here first, then sent to a node with its name, license, the base for an
// adapter, whether the node keeps it after a restart, copies to other nodes and an optional
// rollout with its share of new conversations. Cancel keeps what was sent; sending it again
// resumes. A phone often can't hold a multi-GB model, so the same upload can be shown as the
// `ymirctl` command for a computer that has the file.
import 'dart:async';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart' show TargetPlatform, defaultTargetPlatform;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'models_ui.dart' show humanBytes;
import 'theme.dart';

/// A share typed in the dialog: empty is 0; otherwise a whole percentage from 0 to 100 (`%` allowed).
int? parsePercent(String text) {
  final t = text.trim().replaceAll(RegExp(r'%$'), '').trim();
  if (t.isEmpty) return 0;
  final p = int.tryParse(t);
  return p == null || p < 0 || p > 100 ? null : p;
}

/// Above this a phone is unlikely to hold the file; the dialog suggests the computer command.
const phoneUploadLimit = 4000000000;

bool get _isPhone => defaultTargetPlatform == TargetPlatform.android || defaultTargetPlatform == TargetPlatform.iOS;

/// What the dialog says once the node has it, like the desktop's `result_lines`.
List<String> uploadResultLines(UploadedResultView u, int size, String node) => [
  if (u.alreadyThere)
    '$node already had ${u.id} with the same contents; nothing was sent.'
  else if (u.resumed.toInt() >= size)
    'Stored ${u.id} (${humanBytes(size)}): the node already had all of it from the earlier try.'
  else if (u.resumed > BigInt.zero)
    'Stored ${u.id} (${humanBytes(size)}), resumed after ${humanBytes(u.resumed.toInt())} already there.'
  else
    'Stored ${u.id} (${humanBytes(size)}) and serving it.',
  if (u.saved != null) u.saved!,
  if (u.registerError != null) u.registerError!,
];

class UploadScreen extends StatefulWidget {
  const UploadScreen({super.key, required this.api, required this.nodes, this.resume});

  final BragiApi api;

  /// The nodes from the Models screen; only ready ones take uploads.
  final List<NodeModelsView> nodes;

  /// An unfinished upload to send again (it resumes on the node).
  final UploadSpecView? resume;

  @override
  State<UploadScreen> createState() => _UploadScreenState();
}

class _UploadScreenState extends State<UploadScreen> {
  final _path = TextEditingController();
  final _name = TextEditingController();
  final _license = TextEditingController();
  final _rollout = TextEditingController();
  final _percent = TextEditingController();
  final _file = TextEditingController();
  bool _inspecting = false;
  UploadPreviewView? _preview;
  String? _previewError;
  String? _peer;
  String? _base;
  bool _persist = true;
  String? _inference;
  final Set<String> _copyTo = {};
  String? _run;
  ({int done, int total, String label})? _progress;
  UploadedResultView? _result;
  int _resultSize = 0;
  String? _refused;
  String? _ended;
  String? _problem;
  String? _command;
  StreamSubscription<ModelsEventView>? _sub;

  BragiApi get _api => widget.api;
  List<NodeModelsView> get _ready => widget.nodes.where((n) => n.status == 'ready').toList();
  bool get _sending => _run != null && _result == null && _refused == null && _ended == null;
  bool get _adapter => _preview?.kind == 'adapter';

  @override
  void initState() {
    super.initState();
    final r = widget.resume;
    if (r != null) {
      _path.text = r.path;
      _peer = r.peer;
      _name.text = r.name ?? '';
      _license.text = r.license ?? '';
      _base = r.adapterFor;
      _persist = r.persist;
      _rollout.text = r.rolloutName ?? '';
      _inference = r.rolloutInference;
      _percent.text = r.rolloutName == null ? '' : '${r.rolloutPercent}';
      _copyTo.addAll(r.copyTo);
      _inspect().then((_) {
        if (mounted && _preview != null) _send(r);
      });
    } else if (_ready.length == 1) {
      _peer = _ready.first.node.peer;
    }
    final inf = _inferenceNodes();
    if (_inference == null && inf.isNotEmpty) _inference = inf.first.$1;
  }

  @override
  void dispose() {
    _sub?.cancel();
    for (final c in [_path, _name, _license, _rollout, _percent, _file]) {
      c.dispose();
    }
    super.dispose();
  }

  String _label(String peer) {
    for (final n in widget.nodes) {
      if (n.node.peer == peer) return n.node.label;
    }
    return peer.length > 12 ? peer.substring(0, 12) : peer;
  }

  List<(String, String)> _inferenceNodes() {
    try {
      return [for (final p in _api.modelsInferenceNodes()) (p, _label(p))];
    } catch (_) {
      return const [];
    }
  }

  List<int> _presets() {
    try {
      return _api.uploadPercentPresets();
    } catch (_) {
      return const [0, 5, 20, 50, 100];
    }
  }

  /// The bases an adapter can run on at [peer]: the models served there.
  List<String> _bases(String? peer) => [
    for (final n in widget.nodes.where((n) => n.node.peer == peer))
      for (final m in n.models)
        if (m.base == null && m.servedAs != null) m.servedAs!,
  ];

  Future<void> _choose() async {
    final r = await FilePicker.platform.pickFiles(dialogTitle: 'Model file (GGUF or adapter)');
    final p = r?.files.single.path;
    if (p == null) return;
    _path.text = p;
    await _inspect();
  }

  Future<void> _inspect() async {
    final p = _path.text.trim();
    if (p.isEmpty) return;
    setState(() {
      _inspecting = true;
      _preview = null;
      _previewError = null;
      _problem = null;
      _command = null;
    });
    try {
      final v = await _api.modelsInspectUpload(p);
      if (!mounted) return;
      setState(() {
        _preview = v;
        if (_file.text.isEmpty) _file.text = '~/models/${p.split(RegExp(r'[/\\]')).last}';
      });
    } catch (e) {
      if (mounted) setState(() => _previewError = '$e');
    } finally {
      if (mounted) setState(() => _inspecting = false);
    }
  }

  /// The spec as filled in, or null with [_problem] set.
  UploadSpecView? _spec({bool needFile = true}) {
    String? problem;
    final percent = parsePercent(_percent.text);
    final rollout = _rollout.text.trim();
    if (needFile && _preview == null) {
      problem = 'Pick a file first.';
    } else if (_peer == null) {
      problem = 'Pick the node it goes to.';
    } else if (_adapter && _base == null) {
      problem = 'Pick the base model this adapter runs on.';
    } else if (rollout.isNotEmpty && _inference == null) {
      problem = 'Pick the inference node of the rollout.';
    } else if (rollout.isNotEmpty && percent == null) {
      problem = 'The share is a percentage from 0 to 100, such as 20.';
    }
    setState(() => _problem = problem);
    if (problem != null) return null;
    String? opt(TextEditingController c) => c.text.trim().isEmpty ? null : c.text.trim();
    return UploadSpecView(
      path: _path.text.trim(),
      peer: _peer!,
      name: opt(_name),
      adapterFor: _adapter ? _base : null,
      license: opt(_license),
      persist: _persist,
      rolloutName: rollout.isEmpty ? null : rollout,
      rolloutInference: rollout.isEmpty ? null : _inference,
      rolloutPercent: rollout.isEmpty ? 0 : percent!,
      copyTo: _copyTo.where((p) => p != _peer).toList(),
    );
  }

  Future<void> _start() async {
    final s = _spec();
    if (s != null) await _send(s);
  }

  Future<void> _send(UploadSpecView spec) async {
    // What the last try said goes before this one starts.
    setState(() {
      _result = null;
      _refused = null;
      _ended = null;
      _problem = null;
    });
    await _sub?.cancel();
    try {
      final run = await _api.modelsUpload(spec);
      if (!mounted) return;
      setState(() {
        _run = run;
        _progress = null;
        _result = null;
        _refused = null;
        _ended = null;
        _problem = null;
      });
      _sub = _api.followModels(run).listen((e) {
        if (!mounted) return;
        setState(() {
          switch (e.kind) {
            case 'progress':
              _progress = (done: e.done.toInt(), total: e.total.toInt(), label: e.node?.label ?? '');
            case 'uploaded':
              if (e.upload != null) {
                _result = e.upload;
                _resultSize = e.size.toInt();
                final p = _progress;
                if (p != null) _progress = (done: p.total, total: p.total, label: p.label);
              } else {
                _refused = e.text ?? 'The node refused it.';
              }
            case 'cancelled':
              _ended = 'Cancelled. The node keeps what was sent for 7 days; Resume picks up from there.';
            case 'failed':
              _ended = e.text ?? 'Failed.';
          }
        });
      });
    } catch (e) {
      if (mounted) setState(() => _problem = '$e');
    }
  }

  Future<void> _showCommand() async {
    final s = _spec(needFile: false);
    if (s == null) return;
    final file = _file.text.trim().isEmpty ? '~/models/MODEL.gguf' : _file.text.trim();
    try {
      final c = await _api.uploadCommand(s, file, [for (final n in widget.nodes) n.node]);
      if (mounted) setState(() => _command = c);
    } catch (e) {
      if (mounted) setState(() => _problem = '$e');
    }
  }

  static Widget _small(String t, {Color? color, Key? key}) =>
      Text(t, key: key, style: TextStyle(fontSize: 12, color: color ?? BragiColors.textSecondary));

  Widget _mono(String t, {Key? key}) => Container(
    key: key,
    width: double.infinity,
    margin: const EdgeInsets.symmetric(vertical: 4),
    padding: const EdgeInsets.all(8),
    decoration: BoxDecoration(color: BragiColors.surface, borderRadius: BorderRadius.circular(6)),
    child: Row(
      children: [
        Expanded(child: SelectableText(t, style: const TextStyle(fontSize: 12, fontFamily: 'monospace'))),
        IconButton(
          tooltip: 'Copy',
          visualDensity: VisualDensity.compact,
          icon: const Icon(Icons.copy, size: 18),
          onPressed: () => Clipboard.setData(ClipboardData(text: t)),
        ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    final ready = _ready;
    final preview = _preview;
    final big = preview != null && preview.size.toInt() > phoneUploadLimit && _isPhone;
    final bases = _bases(_peer);
    final inference = _inferenceNodes();
    final busy = _sending;
    return PopScope(
      canPop: !busy,
      child: Scaffold(
        appBar: AppBar(title: const Text('Upload a model')),
        body: ListView(
          key: const Key('upload-list'),
          padding: const EdgeInsets.all(16),
          children: [
            _small('A GGUF model or LoRA adapter on this device goes to one of your model nodes, which serves it.'),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    key: const Key('upload-path'),
                    controller: _path,
                    enabled: !busy,
                    decoration: const InputDecoration(labelText: 'File on this device', isDense: true),
                    onSubmitted: (_) => _inspect(),
                  ),
                ),
                const SizedBox(width: 8),
                OutlinedButton(key: const Key('upload-choose'), onPressed: busy ? null : _choose, child: const Text('Choose…')),
              ],
            ),
            if (_inspecting) Padding(padding: const EdgeInsets.only(top: 8), child: _small('Reading and hashing it here…')),
            if (_previewError != null) Padding(padding: const EdgeInsets.only(top: 8), child: _small(_previewError!, color: BragiColors.error, key: const Key('upload-preview-error'))),
            if (preview != null)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(preview.summary, key: const Key('upload-preview'), style: const TextStyle(fontSize: 13)),
              ),
            if (big)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: _small('That is a lot to send from a phone. Uploading from a computer that has the file is usually quicker: see below.', color: BragiColors.busy, key: const Key('upload-big')),
              ),
            const SizedBox(height: 12),
            DropdownButton<String>(
              key: const Key('upload-node'),
              isExpanded: true,
              hint: const Text('The node it goes to'),
              value: ready.any((n) => n.node.peer == _peer) ? _peer : null,
              items: [for (final n in ready) DropdownMenuItem(value: n.node.peer, child: Text(n.node.label))],
              onChanged: busy ? null : (p) => setState(() {
                _peer = p;
                if (!_bases(p).contains(_base)) _base = null;
              }),
            ),
            if (ready.isEmpty) _small('No model node is ready for uploads.', key: const Key('upload-no-nodes')),
            if (_adapter) ...[
              DropdownButton<String>(
                key: const Key('upload-base'),
                isExpanded: true,
                hint: const Text('The base model it runs on'),
                value: bases.contains(_base) ? _base : null,
                items: [for (final b in bases) DropdownMenuItem(value: b, child: Text(b))],
                onChanged: busy ? null : (b) => setState(() => _base = b),
              ),
              if (_peer != null && bases.isEmpty) _small('That node serves no model to put it on.'),
            ],
            if (ready.length > 1) ...[
              const SizedBox(height: 8),
              _small('Also copy it to:'),
              for (final n in ready.where((n) => n.node.peer != _peer))
                CheckboxListTile(
                  key: Key('upload-copy-${n.node.peer}'),
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  title: Text(n.node.label),
                  value: _copyTo.contains(n.node.peer),
                  onChanged: busy ? null : (v) => setState(() => v == true ? _copyTo.add(n.node.peer) : _copyTo.remove(n.node.peer)),
                ),
              _small('Sent once from this device; the others copy it from that node. A node without room after what it keeps free is left out.'),
            ],
            const SizedBox(height: 8),
            TextField(
              key: const Key('upload-name'),
              controller: _name,
              enabled: !busy,
              decoration: InputDecoration(labelText: 'Name (optional)', hintText: preview?.suggestedId, isDense: true),
            ),
            TextField(
              key: const Key('upload-license'),
              controller: _license,
              enabled: !busy,
              decoration: const InputDecoration(labelText: 'License (optional), such as mit or apache-2.0', isDense: true),
            ),
            SwitchListTile(
              key: const Key('upload-persist'),
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: const Text('Keep it after the node restarts (saved in its node.toml)'),
              value: _persist,
              onChanged: busy ? null : (v) => setState(() => _persist = v),
            ),
            const SizedBox(height: 4),
            Text('ROLLOUT', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
            TextField(
              key: const Key('upload-rollout'),
              controller: _rollout,
              enabled: !busy,
              decoration: const InputDecoration(labelText: 'Add to a rollout (optional): its name, such as support', isDense: true),
              onChanged: (_) => setState(() {}),
            ),
            if (_rollout.text.trim().isNotEmpty) ...[
              DropdownButton<String>(
                key: const Key('upload-inference'),
                isExpanded: true,
                hint: const Text('Its inference node'),
                value: inference.any((n) => n.$1 == _inference) ? _inference : null,
                items: [for (final n in inference) DropdownMenuItem(value: n.$1, child: Text(n.$2))],
                onChanged: busy ? null : (p) => setState(() => _inference = p),
              ),
              Wrap(
                spacing: 6,
                children: [
                  for (final p in _presets())
                    ChoiceChip(
                      key: Key('upload-preset-$p'),
                      label: Text('$p %'),
                      selected: parsePercent(_percent.text) == p,
                      onSelected: busy ? null : (_) => setState(() => _percent.text = '$p'),
                    ),
                ],
              ),
              TextField(
                key: const Key('upload-percent'),
                controller: _percent,
                enabled: !busy,
                keyboardType: TextInputType.number,
                decoration: const InputDecoration(labelText: 'Or type a percentage, 0 to 100', isDense: true),
                onChanged: (_) => setState(() {}),
              ),
              _small('The new version gets exactly this share of new conversations; the other versions share the rest. Empty is 0: pinnable, no traffic yet.'),
            ],
            const SizedBox(height: 12),
            if (_progress != null) ...[
              LinearProgressIndicator(key: const Key('upload-progress'), value: _progress!.total == 0 ? null : _progress!.done / _progress!.total),
              _small('${_progress!.label}: ${humanBytes(_progress!.done)} of ${humanBytes(_progress!.total)}'),
            ],
            if (_result != null) _resultView(_result!),
            if (_refused != null) Text(_refused!, key: const Key('upload-refused'), style: TextStyle(fontSize: 13, color: BragiColors.error)),
            if (_ended != null) _small(_ended!, key: const Key('upload-ended')),
            if (_problem != null) _small(_problem!, color: BragiColors.error, key: const Key('upload-problem')),
            const SizedBox(height: 8),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (busy)
                  TextButton(key: const Key('upload-cancel'), onPressed: () => _api.cancel(_run!), child: const Text('Cancel'))
                else
                  FilledButton(
                    key: const Key('upload-send'),
                    onPressed: preview != null && _peer != null ? _start : null,
                    child: Text(_ended != null ? 'Resume' : 'Upload'),
                  ),
              ],
            ),
            const Divider(height: 32),
            Text('FROM A COMPUTER', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
            _small('When this device can\'t hold the file, run the same upload with ymirctl on a computer that has it.'),
            TextField(
              key: const Key('upload-file-there'),
              controller: _file,
              decoration: const InputDecoration(labelText: 'Its path on that computer', hintText: '~/models/model.gguf', isDense: true),
            ),
            Align(
              alignment: Alignment.centerRight,
              child: TextButton(key: const Key('upload-show-command'), onPressed: _showCommand, child: const Text('Show the command')),
            ),
            if (_command != null) _mono(_command!, key: const Key('upload-command')),
          ],
        ),
      ),
    );
  }

  Widget _resultView(UploadedResultView u) => Column(
    key: const Key('upload-result'),
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final l in uploadResultLines(u, _resultSize, _peer == null ? 'The node' : _label(_peer!))) Text(l, style: const TextStyle(fontSize: 13)),
      for (final c in u.copies) _small(c.ok ? '${c.node}: ${c.text}' : '${c.node}: not copied: ${c.text}', color: c.ok ? null : BragiColors.busy),
      if (u.adapterNote != null) _small(u.adapterNote!, color: BragiColors.busy),
      if (u.rollout != null) Text(u.rollout!, key: const Key('upload-rollout-result'), style: const TextStyle(fontSize: 13)),
      if (u.rolloutError != null) _small('Not added to the rollout: ${u.rolloutError}', color: BragiColors.busy, key: const Key('upload-rollout-error')),
      if (u.rolloutCommands.isNotEmpty) _small('To add it to a rollout with ymirctl:'),
      for (final c in u.rolloutCommands) _mono(c),
    ],
  );
}
