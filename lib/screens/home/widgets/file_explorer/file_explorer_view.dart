import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_grid_view.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_list_view.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

export 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart'
    show ViewMode;

enum Menu {
  download,
  rename,
  sync,
  move,
  copy,
  changelog,
  delete,
  restore,
  deleteForever,
  info,
}

class FileExplorerView extends StatelessWidget {
  const FileExplorerView({super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder<Repository>(
      builder: (repository) {
        return GetBuilder(
          init: FileExplorerController(),
          builder: (fileExplorerController) {
            return FutureBuilder<List<DriveItem>>(
              future: repository.driveService.list(
                repository.fileExplorerViewPath,
              ),
              builder: (context, snapshot) {
                if (!snapshot.hasData) {
                  return Center(child: CircularProgressIndicator());
                }

                final allEntities = snapshot.data!;

                // Sort entities: files first, then folders
                final files = allEntities.whereType<FileMetadata>().toList();
                final folders =
                    allEntities.whereType<FolderMetadata>().toList();

                // Sort each group by name
                files.sort((a, b) => a.name.compareTo(b.name));
                folders.sort((a, b) => a.name.compareTo(b.name));

                // Combine: files first, folders at the end
                final entities = [...files, ...folders];

                return LayoutBuilder(
                  builder: (context, constraints) {
                    return Obx(() {
                      if (fileExplorerController.viewMode.value ==
                          ViewMode.grid) {
                        return FileExplorerGridView(
                          entities: entities,
                          controller: fileExplorerController,
                        );
                      } else {
                        return FileExplorerListView(
                          entities: entities,
                          controller: fileExplorerController,
                          constraints: constraints,
                        );
                      }
                    });
                  },
                );
              },
            );
          },
        );
      },
    );
  }
}
