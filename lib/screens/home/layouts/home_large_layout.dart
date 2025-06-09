import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/get_app_bar.dart';
import 'package:lupine/screens/home/home_controller.dart';

class HomeargeLayout extends StatelessWidget {
  const HomeargeLayout({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: getAppBar(),
      body: GetBuilder<HomeController>(
        builder: (explorerController) {
          return Row(
            children: [
              NavigationRail(
                selectedIndex: HomeController.to.selectedIndex,
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
