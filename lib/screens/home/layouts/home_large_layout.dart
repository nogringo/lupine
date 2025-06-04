import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:window_manager/window_manager.dart';

class HomeargeLayout extends StatelessWidget {
  const HomeargeLayout({super.key});

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
          return Row(
            children: [
              NavigationRail(
                selectedIndex: HomeController.to.selectedIndex,
                groupAlignment: 0,
                labelType: NavigationRailLabelType.selected,
                leading: FloatingActionButton(
                  elevation: 0,
                  onPressed: () {},
                  child: const Icon(Icons.add),
                ),
                destinations:
                    HomeController.destinations
                        .map(
                          (e) => NavigationRailDestination(
                            selectedIcon: Icon(e.selectedIcon),
                            icon: Icon(e.icon),
                            label: Text(e.label),
                          ),
                        )
                        .toList(),
                onDestinationSelected: HomeController.to.onDestinationSelected,
              ),
              Expanded(
                child: Container(
                  margin: EdgeInsets.only(right: 8, bottom: 8),
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.surfaceContainerLow,
                    borderRadius: BorderRadius.circular(16),
                  ),
                  child:
                      HomeController.destinations
                          .map((e) => e.view)
                          .toList()[HomeController.to.selectedIndex],
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}
