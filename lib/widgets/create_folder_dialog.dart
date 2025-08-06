import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:path/path.dart' as p;

class CreateFolderDialog extends StatelessWidget {
  const CreateFolderDialog({super.key});

  @override
  Widget build(BuildContext context) {
    final fieldController = TextEditingController();
    return AlertDialog(
      title: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [Text("Create a new folder"), CloseButton()],
      ),
      content: TextField(
        controller: fieldController,
        decoration: InputDecoration(labelText: "Folder name"),
        autofocus: true,
        inputFormatters: [
          FilteringTextInputFormatter.deny(RegExp(r'[\\/:*?"<>|]')),
        ],
        onSubmitted: (value) {
          final folderPath = p.join(
            Repository.to.fileExplorerViewPath,
            fieldController.text,
          );
          Repository.to.driveService.createFolder(folderPath);
          Get.back();
        },
      ),
      actions: [
        TextButton(
          onPressed: () {
            Get.back();
          },
          child: Text("Cancel"),
        ),
        FilledButton(
          onPressed: () {
            final folderPath = p.join(
              Repository.to.fileExplorerViewPath,
              fieldController.text,
            );
            Repository.to.driveService.createFolder(folderPath);
            Get.back();
          },
          child: Text("Create"),
        ),
      ],
    );
  }
}
