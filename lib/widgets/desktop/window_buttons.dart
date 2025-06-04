import 'package:flutter/material.dart';
import 'package:lupine/constants.dart';
import 'package:window_manager/window_manager.dart';

class WindowButtons extends StatelessWidget {
  const WindowButtons({super.key});

  @override
  Widget build(BuildContext context) {
    if (!isDesktop) return Container();

    return SizedBox(
      width: 138,
      height: 32,
      child: WindowCaption(
        brightness: Theme.of(context).brightness,
        backgroundColor: Colors.transparent,
      ),
    );
  }
}