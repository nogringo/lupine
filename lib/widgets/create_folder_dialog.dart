import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';

class CreateFolderDialog extends StatelessWidget {
  const CreateFolderDialog({super.key});

  @override
  Widget build(BuildContext context) {
    final fieldController = TextEditingController();
    return AlertDialog(
      title: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text("Create a new folder"),
          CloseButton(),
        ],
      ),
      content: TextField(
        controller: fieldController,
        decoration: InputDecoration(labelText: "Folder name"),
        autofocus: true,
        inputFormatters: [
          FilteringTextInputFormatter.deny(
            RegExp(r'[\\/:*?"<>|]'),
          ), // Bloque les caractères interdits
          FilteringTextInputFormatter.deny(
            RegExp(r'^\s|\s$'),
          ), // Bloque les espaces en début/fin
        ],
        onSubmitted: (value) {
          Repository.to.createFolder(fieldController.text);
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
            Repository.to.createFolder(fieldController.text);
            Get.back();
          },
          child: Text("Create"),
        ),
      ],
    );
  }
}
