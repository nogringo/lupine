import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/widgets/folder_selection_dialog/folder_selection_dialog_controller.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class FolderSelectionDialogView extends StatelessWidget {
  final FolderMetadata entity;

  const FolderSelectionDialogView(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder(
      init: FolderSelectionDialogController(entity),
      builder: (c) {
        return AlertDialog(
          scrollable: true,
          content: FutureBuilder<List<FolderMetadata>>(
            future: c.folders,
            builder: (context, snapshot) {
              if (!snapshot.hasData) {
                return Center(child: CircularProgressIndicator());
              }
              final folders = snapshot.data!;
              return Column(
                mainAxisSize: MainAxisSize.min,
                children: List.generate(folders.length, (i) {
                  return ListTile(
                    title: Text(folders[i].name),
                    onTap: () {
                      c.folderPath = folders[i].path;
                    },
                  );
                }),
              );
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
                Get.back();
              },
              child: Text("Move"),
            ),
          ],
        );
      },
    );
  }
}
