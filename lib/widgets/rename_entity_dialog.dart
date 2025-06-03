import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class RenameEntityDialog extends StatelessWidget {
  final DriveEvent entity;

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
          FilteringTextInputFormatter.deny(
            RegExp(r'[\\/:*?"<>|]'),
          ), // Bloque les caractères interdits
          FilteringTextInputFormatter.deny(
            RegExp(r'^\s|\s$'),
          ), // Bloque les espaces en début/fin
        ],
        onSubmitted: (value) {
          Repository.to.renameEntity(entity, fieldController.text);
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
            Repository.to.renameEntity(entity, fieldController.text);
            Get.back();
          },
          child: Text("Rename"),
        ),
      ],
    );
  }
}