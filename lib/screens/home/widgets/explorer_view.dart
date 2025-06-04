import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/file_explorer/file_explorer_view.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/widgets/create_folder_dialog.dart';
import 'package:lupine/widgets/file_explorer_path_view.dart';

class ExplorerView extends StatelessWidget {
  const ExplorerView({super.key});

  @override
  Widget build(BuildContext context) {
    return Column(
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
        Expanded(child: SingleChildScrollView(child: FileExplorerView())),
      ],
    );
  }
}
