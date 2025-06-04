import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:window_manager/window_manager.dart';

class ExplorerSmallLayout extends StatelessWidget {
  const ExplorerSmallLayout({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        surfaceTintColor: Colors.transparent,
        title: () {
          final title = Align(
            alignment: AlignmentDirectional.centerStart,
            child: Text(appTitle),
          );
          if (kIsWeb) return title;
          return DragToMoveArea(child: title);
        }(),
        actions: [
          if (GetPlatform.isDesktop)
            Align(alignment: Alignment.topCenter, child: WindowButtons()),
        ],
      ),
      body: GetBuilder<HomeController>(
        builder: (explorerController) {
          return HomeController.destinations.map((e) => e.view).toList()[HomeController.to.selectedIndex];
        },
      ),
      bottomNavigationBar: GetBuilder<HomeController>(
        builder: (explorerController) {
          return NavigationBar(
            selectedIndex: HomeController.to.selectedIndex,
            labelBehavior: NavigationDestinationLabelBehavior.onlyShowSelected,
            destinations: HomeController.destinations.map((e) => NavigationDestination(
                selectedIcon: Icon(e.selectedIcon),
                icon: Icon(e.icon),
                label: e.label,
              )).toList(),
            onDestinationSelected: HomeController.to.onDestinationSelected,
          );
        },
      ),
    );
  }
}
