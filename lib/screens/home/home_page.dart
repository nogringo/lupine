import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/screens/home/layouts/explorer_large_layout.dart';
import 'package:lupine/screens/home/layouts/explorer_small_layout.dart';
import 'package:lupine/screens/home/widgets/drop_zone/drop_zone_view.dart';

class HomePage extends StatelessWidget {
  const HomePage({super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(HomeController());
    return DropZoneView(
      child: LayoutBuilder(builder: (context, constraints) {
        if (constraints.maxWidth > maxSmallWidth) return ExplorerLargeLayout();
        return ExplorerSmallLayout();
      }),
    );
  }
}
