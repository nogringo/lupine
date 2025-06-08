import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/screens/home/layouts/home_large_layout.dart';
import 'package:lupine/screens/home/layouts/home_small_layout.dart';
import 'package:lupine/screens/home/widgets/drop_zone/drop_zone_view.dart';
import 'package:window_manager/window_manager.dart';

class HomePage extends StatelessWidget {
  const HomePage({super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(HomeController());

    if (GetPlatform.isMobile) {
      return HomeSmallLayout();
    }

    final child = DropZoneView(
      child: LayoutBuilder(
        builder: (context, constraints) {
          if (constraints.maxWidth > maxSmallWidth) return HomeargeLayout();
          return HomeSmallLayout();
        },
      ),
    );

    if (isDesktop) return DragToResizeArea(child: child);
    return child;
  }
}
