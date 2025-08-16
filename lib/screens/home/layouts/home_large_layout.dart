import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/get_app_bar.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/widgets/sidebar/sidebar_controller.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/widgets/create_folder_dialog.dart';

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
                          child: FloatingActionButton(
                            heroTag: "fab_hidden",
                            onPressed: null,
                          ),
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
                          heroTag: "fab_menu",
                          onPressed: () {
                            showMenu(
                              context: context,
                              position: RelativeRect.fromLTRB(20, 80, 200, 0),
                              shape: RoundedRectangleBorder(
                                borderRadius: BorderRadius.circular(12),
                                side: BorderSide(
                                  color:
                                      Theme.of(
                                        context,
                                      ).colorScheme.outlineVariant,
                                  width: 1,
                                ),
                              ),
                              elevation: 8,
                              items: [
                                PopupMenuItem(
                                  child: Row(
                                    children: [
                                      Icon(Icons.create_new_folder_outlined),
                                      SizedBox(width: 12),
                                      Text('New Folder'),
                                    ],
                                  ),
                                  onTap: () {
                                    Future.delayed(Duration.zero, () {
                                      Get.dialog(CreateFolderDialog());
                                    });
                                  },
                                ),
                                PopupMenuItem(
                                  child: Row(
                                    children: [
                                      Icon(Icons.upload_file),
                                      SizedBox(width: 12),
                                      Text('Upload Files'),
                                    ],
                                  ),
                                  onTap: () {
                                    Repository.to.pickFiles();
                                  },
                                ),
                                PopupMenuItem(
                                  child: Row(
                                    children: [
                                      Icon(Icons.drive_folder_upload_outlined),
                                      SizedBox(width: 12),
                                      Text('Upload Folder'),
                                    ],
                                  ),
                                  onTap: () {
                                    Repository.to.pickFolder();
                                  },
                                ),
                              ],
                            );
                          },
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
