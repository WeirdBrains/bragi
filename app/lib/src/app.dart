import 'package:material_ui/material_ui.dart';

import 'chat_screen.dart';
import 'controller.dart';
import 'pairing_screen.dart';
import 'theme.dart';

class BragiApp extends StatefulWidget {
  const BragiApp({super.key, required this.controller});

  final BragiController controller;

  @override
  State<BragiApp> createState() => _BragiAppState();
}

class _BragiAppState extends State<BragiApp> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    widget.controller.appearance.systemDark = WidgetsBinding.instance.platformDispatcher.platformBrightness == Brightness.dark;
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  // Mobile lifecycle: the node pauses in the background and resumes in the foreground.
  @override
  void didChangePlatformBrightness() {
    widget.controller.appearance.platformBrightness(View.of(context).platformDispatcher.platformBrightness == Brightness.dark);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState s) {
    final api = widget.controller.api;
    if (s == AppLifecycleState.paused) api.paused().ignore();
    if (s == AppLifecycleState.resumed) api.resumed().ignore();
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: widget.controller.appearance,
      builder: (context, _) => MaterialApp(
        title: 'Bragi',
        debugShowCheckedModeBanner: false,
        theme: bragiTheme(),
        home: ListenableBuilder(
          listenable: widget.controller,
          builder: (_, _) => widget.controller.linked ? ChatScreen(controller: widget.controller) : PairingScreen(controller: widget.controller),
        ),
      ),
    );
  }
}
