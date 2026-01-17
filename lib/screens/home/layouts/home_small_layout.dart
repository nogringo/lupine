import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/get_app_bar.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/widgets/upload_panel/upload_panel_view.dart';

class HomeSmallLayout extends StatelessWidget {
  const HomeSmallLayout({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: getAppBar(),
      body: Stack(
        children: [
          GetBuilder<HomeController>(
            builder: (explorerController) {
              return HomeController.destinations
                  .map((e) => e.view)
                  .toList()[HomeController.to.selectedIndex];
            },
          ),
          const UploadPanelView(),
        ],
      ),
      bottomNavigationBar: GetBuilder<HomeController>(
        builder: (explorerController) {
          return NavigationBar(
            selectedIndex: HomeController.to.selectedIndex,
            labelBehavior: NavigationDestinationLabelBehavior.onlyShowSelected,
            destinations: HomeController.destinations
                .map(
                  (e) => NavigationDestination(
                    selectedIcon: Icon(e.selectedIcon),
                    icon: Icon(e.icon),
                    label: e.label,
                  ),
                )
                .toList(),
            onDestinationSelected: HomeController.to.onDestinationSelected,
          );
        },
      ),
    );
  }
}
