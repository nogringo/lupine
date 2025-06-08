import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/screens/login/login_page.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:mime/mime.dart';
import 'package:path/path.dart' as p;
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:toastification/toastification.dart';

class Repository extends GetxController {
  static Repository get to => Get.find();

  final storage = FlutterSecureStorage();

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

  List<DriveEvent> get driveEvents => DriveService().driveEvents;

  login(String privkey) {
    DriveService().login(privkey: privkey);
    listenEvents();
  }

  void listenEvents() async {
    DriveService().updateEvents.listen((_) => update());
  }

  logout() async {
    fileExplorerViewPath = "/MyFiles";
    DriveService().logout();
    await Repository.to.storage.delete(key: "privkey");

    Get.offAll(() => LoginPage());
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
    addFolder(selectedDirectory, fileExplorerViewPath);
  }

  Future<void> addFolder(String path, String destPath) async {
    DriveService().addFolder(path, destPath);
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

      DriveService().addFile(
        bytes: bytes,
        name: file.name,
        mimeType: lookupMimeType(file.path!),
        destPath: fileExplorerViewPath,
      );
    }
  }

  void listBlobs() async {
    // try {
    //   final responses = await ndk.blossom.listBlobs(
    //     pubkey:
    //         "0ca3f123c42ba503f7dc5962f3768ca0c9ae36806f8aa96543e28cc8f24ce9b5",
    //     serverUrls: blossomServers,
    //   );

    //   for (var response in responses) {
    //     print(response.sha256);
    //   }
    // } catch (e) {
    //   print(e);
    // }
  }

  void checkBlob() async {
    // try {
    //   final response = await ndk.blossom.checkBlob(
    //     sha256:
    //         "fdce6b78e828130971bfd21c3ee87d4dd2c67af20b30c9ab90ab297c1036b8d1",
    //     serverUrls: blossomServers,
    //   );

    //   print(response);
    // } catch (e) {
    //   print(e);
    // }
  }

  void createFolder(String name, {String? destPath}) async {
    destPath ??= fileExplorerViewPath;

    DriveService().createFolder(name, destPath: destPath);
  }

  void deleteEvents(List<String> eventsId) async {
    DriveService().deleteEvents(eventsId);
  }

  void copyEntityTo(DriveEvent driveEvent, String toPath) async {
    DriveService().copyEntityTo(driveEvent, toPath);
  }

  void moveEntityTo(DriveEvent driveEvent, String toPath) {
    DriveService().moveEntityTo(driveEvent, toPath);
  }

  void deleteEntity(DriveEvent entity) {
    if (p.isWithin(trashPath, entity.path)) {
      DriveService().deleteEntity(entity);
      return;
    }

    DriveService().moveEntityTo(entity, trashPath);
  }

  void deleteEntityForever(DriveEvent entity) {
    DriveService().deleteEntity(entity);
  }

  void emptyTrash() {
    final entitiesToDelete = DriveService().list(trashPath);
    for (var entity in entitiesToDelete) {
      DriveService().deleteEntity(entity);
    }
  }

  renameEntity(DriveEvent entity, String newName) async {
    DriveService().renameEntity(entity, newName);
  }
}
