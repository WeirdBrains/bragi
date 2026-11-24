// Training consent for the open conversation (Ymir's training plan P2): off unless the user
// turns it on, and it syncs to every linked device. Turning it on asks first and says what it
// means; turning it off withdraws it at once. The record itself is bragi-core's.
import 'package:material_ui/material_ui.dart';

import 'controller.dart';
import 'theme.dart';

/// The app bar button: a school cap, filled while the conversation is marked for training.
class TrainingToggle extends StatelessWidget {
  const TrainingToggle({super.key, required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    final cur = controller.current;
    if (cur == null) return const SizedBox.shrink();
    final on = cur.training;
    return IconButton(
      key: Key(on ? 'training-on' : 'training-off'),
      // No tooltip: its long-press overlay got in the way of the message menu's dialogs.
      icon: Icon(
        on ? Icons.school : Icons.school_outlined,
        color: on ? BragiColors.accent : null,
        semanticLabel: on ? 'Marked for training (tap to withdraw)' : 'Not used for training',
      ),
      onPressed: controller.streaming ? null : () => toggleTraining(context, controller),
    );
  }
}

/// Withdraws at once; asks before marking.
Future<void> toggleTraining(BuildContext context, BragiController c) async {
  final cur = c.current;
  if (cur == null) return;
  if (cur.training) {
    await c.setTraining(false);
    return;
  }
  final ok = await showDialog<bool>(
    context: context,
    builder: (d) => AlertDialog(
      title: const Text('Use this conversation for training?'),
      content: const Text(
        'Your Ymir nodes may export it as training data for your own models, on your own devices. '
        'System prompts, attachments, tool results and reasoning are left out. '
        'You can withdraw it any time; a dataset already exported keeps a list so it can be rebuilt without it.',
      ),
      actions: [
        TextButton(key: const Key('training-cancel'), onPressed: () => Navigator.of(d).pop(false), child: const Text('Cancel')),
        FilledButton(key: const Key('training-confirm'), onPressed: () => Navigator.of(d).pop(true), child: const Text('Use for training')),
      ],
    ),
  );
  if (ok == true) await c.setTraining(true);
}
