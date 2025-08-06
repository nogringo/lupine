import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/get_app_bar.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/widgets/sidebar/sidebar_controller.dart';

class HomeargeLayout extends StatelessWidget {
  const HomeargeLayout({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: getAppBar(),
      body: GetBuilder<HomeController>(
        builder: (homeController) {
          return Row(
            children: [
              GetBuilder<SidebarController>(
                init: SidebarController(),
                builder: (sidebarController) {
                  return Stack(
                    children: [
                      NavigationRail(
                        leading: Opacity(
                          opacity: 0,
                          child: FloatingActionButton(onPressed: null),
                        ),
                        selectedIndex: HomeController.to.selectedIndex,
                        extended: sidebarController.extended,
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
                        onDestinationSelected:
                            HomeController.to.onDestinationSelected,
                      ),
                      Positioned(
                        left: 9,
                        top: 8,
                        child: FloatingActionButton.extended(
                          onPressed: () {},
                          label: Text("New"),
                          icon: Icon(Icons.add),
                          isExtended: sidebarController.extended,
                        ),
                      ),
                      Positioned(
                        right: 8,
                        bottom: 8,
                        child: IconButton(
                          onPressed: () {
                            sidebarController.toogleExtend();
                          },
                          icon: Icon(
                            sidebarController.extended
                                ? Icons.arrow_back_ios_rounded
                                : Icons.arrow_forward_ios_rounded,
                          ),
                        ),
                      ),
                    ],
                  );
                },
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
