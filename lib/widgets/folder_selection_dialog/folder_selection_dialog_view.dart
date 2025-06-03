import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/widgets/folder_selection_dialog/folder_selection_dialog_controller.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class FolderSelectionDialogView extends StatelessWidget {
  final DriveEvent entity;

  const FolderSelectionDialogView(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder(
      init: FolderSelectionDialogController(entity),
      builder: (c) {
        return AlertDialog(
          scrollable: true,
          content: Column(
            children: List.generate(c.folders.length, (i) {
              return ListTile(
                title: Text(c.folders[i].name),
                onTap: () {
                  
                },
              );
            }),
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
      }
    );
  }
}