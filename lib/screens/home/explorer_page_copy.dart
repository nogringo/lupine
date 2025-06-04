import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/file_explorer/file_explorer_view.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/home_controller.dart';
import 'package:lupine/widgets/create_folder_dialog.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:lupine/widgets/file_explorer_path_view.dart';
import 'package:window_manager/window_manager.dart';

class ExplorerPageCopy extends StatelessWidget {
  const ExplorerPageCopy({super.key});

  @override
  Widget build(BuildContext context) {
    return Builder(
      builder: (context) {
        return DropTarget(
          onDragDone: (detail) {
            for (var file in detail.files) {
              final entity = FileSystemEntity.typeSync(file.path);

              if (entity == FileSystemEntityType.directory) {
                Repository.to.addFolder(
                  file.path,
                  Repository.to.fileExplorerViewPath,
                );
              } else if (entity == FileSystemEntityType.file) {
                Repository.to.addFile(file.path);
              }
            }
          },
          onDragEntered: (detail) {
            Repository.to.isDraggingFile = true;
          },
          onDragExited: (detail) {
            Repository.to.isDraggingFile = false;
          },
          child: GetBuilder(
            init: Repository.to,
            builder: (repository) {
              if (repository.isDraggingFile) {
                return Scaffold(
                  body: Center(
                    child: Text(
                      "Drop here",
                      style: Theme.of(context).textTheme.displayLarge,
                    ),
                  ),
                );
              }
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
                      Align(
                        alignment: Alignment.topCenter,
                        child: WindowButtons(),
                      ),
                  ],
                ),
                body: GetBuilder(
                  init: HomeController(),
                  builder: (explorerController) {
                    return Row(
                      children: [
                        NavigationRail(
                          leading: FloatingActionButton(
                            elevation: 0,
                            onPressed: () {},
                            child: const Icon(Icons.add),
                          ),
                          destinations: [
                            NavigationRailDestination(
                              icon: Icon(Icons.home_outlined),
                              selectedIcon: Icon(Icons.home),
                              label: Text("Files"),
                            ),
                            NavigationRailDestination(
                              icon: Icon(Icons.star_outline),
                              selectedIcon: Icon(Icons.star),
                              label: Text("Favoris"),
                            ),
                            NavigationRailDestination(
                              icon: Icon(Icons.delete_outlined),
                              selectedIcon: Icon(Icons.delete),
                              label: Text("Trash"),
                            ),
                            NavigationRailDestination(
                              icon: Icon(Icons.settings_outlined),
                              selectedIcon: Icon(Icons.settings),
                              label: Text("Settings"),
                            ),
                          ],
                          selectedIndex: HomeController.to.selectedIndex,
                          groupAlignment: 0,
                          labelType: NavigationRailLabelType.selected,
                          onDestinationSelected: HomeController.to.onDestinationSelected,
                        ),
                        Expanded(
                          child: Container(
                            margin: EdgeInsets.only(right: 8, bottom: 8),
                            decoration: BoxDecoration(
                              color:
                                  Theme.of(context).colorScheme.surfaceContainerLow,
                              borderRadius: BorderRadius.circular(16),
                            ),
                            child: Column(
                              children: [
                                Padding(
                                  padding: const EdgeInsets.all(8.0),
                                  child: Row(
                                    children: [
                                      Expanded(child: FileExplorerPathView()),
                                      IconButton(
                                        onPressed: () {
                                          Get.dialog(CreateFolderDialog());
                                        },
                                        icon: Icon(
                                          Icons.create_new_folder_outlined,
                                        ),
                                      ),
                                      IconButton(
                                        onPressed: repository.pickFolder,
                                        icon: Icon(
                                          Icons.drive_folder_upload_outlined,
                                        ),
                                      ),
                                      IconButton(
                                        onPressed: repository.pickFiles,
                                        icon: Icon(Icons.upload_file),
                                      ),
                                    ],
                                  ),
                                ),
                                Expanded(
                                  child: SingleChildScrollView(
                                    child: FileExplorerView(),
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ],
                    );
                  }
                ),
              );
            },
          ),
        );
      },
    );
  }
}
