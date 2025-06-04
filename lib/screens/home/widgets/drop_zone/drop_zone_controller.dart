import 'dart:io';

import 'package:lupine/repository.dart';

class DropZoneController {
  static onDragDone(detail) {
    for (var file in detail.files) {
      final entity = FileSystemEntity.typeSync(file.path);

      if (entity == FileSystemEntityType.directory) {
        Repository.to.addFolder(file.path, Repository.to.fileExplorerViewPath);
      } else if (entity == FileSystemEntityType.file) {
        Repository.to.addFile(file.path);
      }
    }
  }

  static onDragEntered(detail) {
    Repository.to.isDraggingFile = true;
  }

  static onDragExited(detail) {
    Repository.to.isDraggingFile = false;
  }
}
