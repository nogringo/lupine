import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/config.dart';
import 'package:lupine/get_database.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:mime/mime.dart';
import 'package:ndk/ndk.dart';
import 'package:path/path.dart' as p;
import 'package:sembast_cache_manager/sembast_cache_manager.dart';
import 'package:toastification/toastification.dart';

class Repository extends GetxController {
  static Repository get to => Get.find();

  late Ndk ndk;
  late DriveService driveService;

  bool isAppLoaded = false;

  String _fileExplorerViewPath = "/MyFiles";

  bool _isDraggingFile = false;

  bool get isDraggingFile => _isDraggingFile;
  set isDraggingFile(bool value) {
    _isDraggingFile = value;
    update();
  }

  String get fileExplorerViewPath => _fileExplorerViewPath;
  set fileExplorerViewPath(String value) {
    _fileExplorerViewPath = value;
    update();
  }

  Future<void> init() async {
    if (isAppLoaded) return;

    ndk = Ndk(
      NdkConfig(
        eventVerifier: Bip340EventVerifier(),
        cache: SembastCacheManager(await getDatabase()),
      ),
    );

    driveService = DriveService(ndk: ndk, db: await getDatabase("lupine"));

    listenEvents();

    isAppLoaded = true;
  }

  void listenEvents() async {
    Repository.to.driveService.changes.listen((change) {
      print("${change.type} and ${change.path}");
      update();
    });
  }

  logout() async {
    fileExplorerViewPath = "/MyFiles";
    Get.offAllNamed(AppRoutes.login);
  }

  void pickFolder() async {
    if (kIsWeb) {
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
      return;
    }

    String? selectedDirectory = await FilePicker.platform.getDirectoryPath();
    if (selectedDirectory == null) return;
    uploadFolder(folderPath: selectedDirectory);
  }

  void pickFiles() async {
    FilePickerResult? result = await FilePicker.platform.pickFiles(
      allowMultiple: true,
    );

    if (result == null) return;
    for (var file in result.files) {
      Uint8List? bytes;
      if (kIsWeb) {
        bytes = file.bytes;
      } else {
        bytes = await File(file.path!).readAsBytes();
      }

      if (bytes == null) continue;

      driveService.uploadFile(
        fileData: bytes,
        path: p.join(fileExplorerViewPath, file.name),
        fileType: lookupMimeType(file.path!),
      );
    }
  }

  void moveToTrash(String path) {
    if (p.isWithin(trashPath, path)) return;
    final fileName = p.basename(path);
    final newPath = p.join(trashPath, fileName);
    driveService.move(oldPath: path, newPath: newPath);
  }

  void emptyTrash() async {
    final entitiesToDelete = await driveService.list(trashPath);
    for (var entity in entitiesToDelete) {
      driveService.deleteByPath(entity.path);
    }
  }

  Future<void> uploadFolder({
    required String folderPath,
    String? destPath,
  }) async {
    if (kIsWeb) return;

    final directory = Directory(folderPath);
    if (!await directory.exists()) {
      throw Exception('Folder does not exist: $folderPath');
    }

    final targetPath = destPath ?? fileExplorerViewPath;
    final folderName = p.basename(folderPath);
    final newFolderPath = p.join(targetPath, folderName);

    await driveService.createFolder(newFolderPath);

    await for (var entity in directory.list(recursive: false)) {
      final entityName = p.basename(entity.path);

      if (entity is File) {
        final bytes = await entity.readAsBytes();
        final mimeType = lookupMimeType(entity.path);

        await driveService.uploadFile(
          fileData: bytes,
          path: p.join(newFolderPath, entityName),
          fileType: mimeType,
        );
      } else if (entity is Directory) {
        await uploadFolder(folderPath: entity.path, destPath: newFolderPath);
      }
    }
  }
}
