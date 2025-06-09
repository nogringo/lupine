import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:mime/mime.dart';
import 'package:path/path.dart' as p;
import 'package:toastification/toastification.dart';

class DropZoneController {
  static onDragDone(DropDoneDetails detail) async {
    if (kIsWeb) {
      for (var file in detail.files) {
        final isDirectory = file.mimeType == "directory";
        if (isDirectory) {
          toastification.show(
            context: Get.context,
            type: ToastificationType.custom(
              "name",
              Theme.of(Get.context!).colorScheme.primaryContainer,
              Icons.info_outlined,
            ),
            style: ToastificationStyle.fillColored,
            alignment: Alignment.bottomRight,
            title: Text("Drop folder"),
            description: Text("$appTitle web does not support folder drop"),
          );
          continue; // TODO Allow folder drop on web
        }

        DriveService().addFile(
          bytes: await file.readAsBytes(),
          name: file.name,
          mimeType: file.mimeType,
          destPath: Repository.to.fileExplorerViewPath,
        );
      }
      return;
    }

    for (var file in detail.files) {
      final entityType = FileSystemEntity.typeSync(file.path);
      if (entityType == FileSystemEntityType.directory) {
        Repository.to.addFolder(file.path, Repository.to.fileExplorerViewPath);
      } else if (entityType == FileSystemEntityType.file) {
        DriveService().addFile(
          bytes: await file.readAsBytes(),
          name: p.basename(file.path),
          mimeType: lookupMimeType(file.path),
          destPath: Repository.to.fileExplorerViewPath,
        );
      }
    }
  }

  static onDragEntered(DropEventDetails detail) {
    Repository.to.isDraggingFile = true;
  }

  static onDragExited(DropEventDetails detail) {
    Repository.to.isDraggingFile = false;
  }
}
