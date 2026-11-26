// The training view (Ymir's training plan, after Bragi v1; the per-chat consent toggle is training_ui.dart): each of your Ymir model nodes' training
// runs and resumable jobs, and the versioned rollouts on your inference nodes with each version's
// share, the canary and the run that trained it. One core call (`training_overview`); a read.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'controller.dart';
import 'theme.dart';

/// `2026-11-25 12:00 UTC` from Unix milliseconds.
String utcMinute(int ms) {
  final t = DateTime.fromMillisecondsSinceEpoch(ms, isUtc: true);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${t.year}-${two(t.month)}-${two(t.day)} ${two(t.hour)}:${two(t.minute)} UTC';
}

Color stageColor(String stage) => switch (stage) {
  'live' => BragiColors.warm,
  'canary' => BragiColors.busy,
  _ => BragiColors.textSecondary,
};

class TrainingScreen extends StatefulWidget {
  const TrainingScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  State<TrainingScreen> createState() => _TrainingScreenState();
}

class _TrainingScreenState extends State<TrainingScreen> {
  TrainingOverviewView? _o;
  String? _error;

  @override
  void initState() {
    super.initState();
    _reload();
  }

  Future<void> _reload() async {
    try {
      final o = await widget.controller.api.trainingOverview();
      if (mounted) {
        setState(() {
          _o = o;
          _error = null;
        });
      }
    } catch (e) {
      if (mounted) setState(() => _error = '$e');
    }
  }

  Widget _heading(String s) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 16, 16, 4),
    child: Text(s, style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
  );

  Widget _small(String s, {Key? key}) => Text(
    s,
    key: key,
    style: TextStyle(fontSize: 12, color: BragiColors.textSecondary),
  );

  Widget _rollout(RolloutView r) => Card(
    key: Key('rollout-${r.node.peer}-${r.name}'),
    margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
    child: Padding(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('${r.name} on ${r.node.label}', style: const TextStyle(fontSize: 14)),
          for (final v in r.versions)
            Row(
              children: [
                Expanded(
                  child: Text(
                    'v${v.version} · ${v.model} · ${v.percent.toStringAsFixed(0)}%'
                    '${v.trainedOn != null ? ' · trained on ${v.trainedOn}, run ${v.trainedRun}' : ''}',
                    style: const TextStyle(fontSize: 13),
                  ),
                ),
                Text(
                  v.stage,
                  key: Key('stage-${r.name}-${v.version}'),
                  style: TextStyle(fontSize: 12, color: stageColor(v.stage)),
                ),
              ],
            ),
        ],
      ),
    ),
  );

  Widget _node(NodeTrainingView n) => Card(
    key: Key('training-node-${n.node.peer}'),
    margin: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
    child: Padding(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(child: Text(n.node.label, style: const TextStyle(fontSize: 14))),
              _small(n.remote ? 'takes runs from your devices' : 'trains locally only'),
            ],
          ),
          if (n.runs.isEmpty && n.jobs.isEmpty) _small('No training runs.'),
          for (final j in n.jobs)
            Text(
              'job ${j.id} · ${j.state}${j.holder != null ? ' on ${j.holder}' : ''} · ${j.base} → ${j.adapter}'
              ' (attempt ${j.attempts}${j.checkpointEpoch != null ? ', checkpoint after epoch ${j.checkpointEpoch}' : ''})',
              style: const TextStyle(fontSize: 13),
            ),
          for (final r in n.runs) ...[
            Text(
              '${utcMinute(r.startedMs)} · ${r.status} · ${r.base}${r.adapter != null ? ' → ${r.adapter}' : ''}'
              '${r.finalLoss != null ? ' · loss ${r.finalLoss!.toStringAsFixed(3)}' : ''}',
              key: Key('run-${r.id}'),
              style: const TextStyle(fontSize: 13),
            ),
            _small('${r.rowsUsed} rows of ${r.dataset} (${r.datasetLicense}), ${r.epochs} epochs'),
            if (r.error != null) Text(r.error!, style: TextStyle(fontSize: 11, color: BragiColors.error)),
          ],
        ],
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final o = _o;
    final ready = o?.nodes.where((n) => n.status == 'ready').toList() ?? const <NodeTrainingView>[];
    final others = (o?.nodes.length ?? 0) - ready.length;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Training'),
        actions: [IconButton(key: const Key('training-refresh'), tooltip: 'Refresh', icon: const Icon(Icons.refresh), onPressed: _reload)],
      ),
      body: ListView(
        key: const Key('training-list'),
        padding: const EdgeInsets.only(bottom: 24),
        children: [
          if (_error != null)
            Padding(
              padding: const EdgeInsets.all(16),
              child: Text(_error!, style: TextStyle(color: BragiColors.error)),
            ),
          if (o == null && _error == null) Padding(padding: const EdgeInsets.all(16), child: _small('Asking your devices…')),
          if (o != null) ...[
            _heading('ROLLOUTS'),
            if (o.rollouts.isEmpty)
              Padding(padding: const EdgeInsets.symmetric(horizontal: 16), child: _small('No versioned rollouts on your inference nodes.')),
            for (final r in o.rollouts) _rollout(r),
            _heading('RUNS AND JOBS'),
            if (ready.isEmpty)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 16),
                child: _small('None of your devices reports training (it needs a Ymir model node, 0.15.5 or later).'),
              ),
            for (final n in ready) _node(n),
            if (others > 0)
              Padding(
                padding: const EdgeInsets.all(16),
                child: _small('$others other devices report no training.', key: const Key('training-others')),
              ),
          ],
        ],
      ),
    );
  }
}
