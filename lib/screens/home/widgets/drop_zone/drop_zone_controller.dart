import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/controllers/upload_queue_controller.dart';
import 'package:lupine/repository.dart';
import 'package:mime/mime.dart';
import 'package:path/path.dart' as p;
import 'package:toastification/toastification.dart';

class DropZoneController {
  static Future<void> onDragDone(DropDoneDetails detail) async {
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

        UploadQueueController.to.enqueue(
          fileName: file.name,
          destPath: p.join(Repository.to.fileExplorerViewPath, file.name),
          fileData: await file.readAsBytes(),
          mimeType: file.mimeType,
        );
      }
      return;
    }

    for (var file in detail.files) {
      final entityType = FileSystemEntity.typeSync(file.path);
      if (entityType == FileSystemEntityType.directory) {
        Repository.to.uploadFolder(folderPath: file.path);
      } else if (entityType == FileSystemEntityType.file) {
        UploadQueueController.to.enqueue(
          fileName: p.basename(file.path),
          destPath: p.join(
            Repository.to.fileExplorerViewPath,
            p.basename(file.path),
          ),
          fileData: await file.readAsBytes(),
          mimeType: lookupMimeType(file.path),
        );
      }
    }
  }

  static void onDragEntered(DropEventDetails detail) {
    Repository.to.isDraggingFile = true;
  }

  static void onDragExited(DropEventDetails detail) {
    Repository.to.isDraggingFile = false;
  }
}
