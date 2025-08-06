import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_view.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/widgets/create_folder_dialog.dart';
import 'package:lupine/widgets/file_explorer_path_view.dart';

class ExplorerView extends StatelessWidget {
  const ExplorerView({super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder<FileExplorerController>(
      init: FileExplorerController(),
      builder: (fileExplorerController) {
        return Column(
          children: [
            Padding(
              padding: const EdgeInsets.all(8.0),
              child: Row(
                children: [
                  Expanded(child: FileExplorerPathView()),
                  Obx(
                    () => IconButton(
                      icon: Icon(
                        fileExplorerController.viewMode.value == ViewMode.list
                            ? Icons.grid_view_rounded
                            : Icons.list_rounded,
                      ),
                      onPressed: fileExplorerController.toggleViewMode,
                      tooltip:
                          fileExplorerController.viewMode.value == ViewMode.list
                              ? 'Grid view'
                              : 'List view',
                    ),
                  ),
                  IconButton(
                    onPressed: () {
                      Get.dialog(CreateFolderDialog());
                    },
                    icon: Icon(Icons.create_new_folder_outlined),
                  ),
                  IconButton(
                    onPressed: Repository.to.pickFolder,
                    icon: Icon(Icons.drive_folder_upload_outlined),
                  ),
                  IconButton(
                    onPressed: Repository.to.pickFiles,
                    icon: Icon(Icons.upload_file),
                  ),
                ],
              ),
            ),
            Expanded(child: FileExplorerView()),
          ],
        );
      },
    );
  }
}
