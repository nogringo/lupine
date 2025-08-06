import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:path/path.dart' as p;

class RenameEntityDialog extends StatelessWidget {
  final DriveItem entity;

  const RenameEntityDialog(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    final fieldController = TextEditingController(text: entity.name);
    return AlertDialog(
      title: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(entity.isFile ? "Rename a file" : "Rename a folder"),
          SizedBox(width: 32),
          CloseButton(),
        ],
      ),
      content: TextField(
        controller: fieldController,
        decoration: InputDecoration(labelText: "Name"),
        autofocus: true,
        inputFormatters: [
          FilteringTextInputFormatter.deny(RegExp(r'[\\/:*?"<>|]')),
          FilteringTextInputFormatter.deny(RegExp(r'^\s|\s$')),
        ],
        onSubmitted: (value) {
          final directory = p.dirname(entity.path);
          final newPath = p.join(directory, fieldController.text);
          Repository.to.driveService.move(
            oldPath: entity.path,
            newPath: newPath,
          );
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
            final directory = p.dirname(entity.path);
            final newPath = p.join(directory, fieldController.text);
            Repository.to.driveService.move(
              oldPath: entity.path,
              newPath: newPath,
            );
            Get.back();
          },
          child: Text("Rename"),
        ),
      ],
    );
  }
}
