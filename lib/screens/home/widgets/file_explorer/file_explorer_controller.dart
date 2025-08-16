import 'package:file_picker/file_picker.dart';
import 'package:file_saver/file_saver.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:toastification/toastification.dart';
import 'package:lupine/screens/home/widgets/changelog_dialog/changelog_dialog_view.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_view.dart';
import 'package:lupine/widgets/folder_selection_dialog/folder_selection_dialog_view.dart';
import 'package:lupine/widgets/info_dialog.dart';
import 'package:lupine/widgets/rename_entity_dialog.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:path/path.dart' as p;

class FileExplorerController extends GetxController {
  static FileExplorerController get to => Get.find();

  int _sortColumnIndex = 0;
  bool _sortAscending = true;

  int get sortColumnIndex => _sortColumnIndex;
  set sortColumnIndex(int value) {
    _sortColumnIndex = value;
    update();
  }

  bool get sortAscending => _sortAscending;
  set sortAscending(bool value) {
    _sortAscending = value;
    update();
  }

  Future<List<DriveItem>> get entities async {
    List<int Function(DriveItem, DriveItem)> sortByColumnAsc = [
      (a, b) => a.name.compareTo(b.name),
      (a, b) => a.createdAt.compareTo(b.createdAt),
      // (a, b) => a.size.compareTo(b.size),
    ];

    List<int Function(DriveItem, DriveItem)> sortByColumnDec = [
      (a, b) => b.name.compareTo(a.name),
      (a, b) => b.createdAt.compareTo(a.createdAt),
      // (a, b) => b.size.compareTo(a.size),
    ];

    final driveItems = await Repository.to.driveService.list(
      Repository.to.fileExplorerViewPath,
    );

    driveItems.sort(
      sortAscending
          ? sortByColumnAsc[sortColumnIndex]
          : sortByColumnDec[sortColumnIndex],
    );
    return driveItems;
  }

  void onSort(int columnIndex, bool ascending) {
    sortColumnIndex = columnIndex;
    sortAscending = ascending;
  }

  void onEntityMenuSelected(Menu value, DriveItem entity) async {
    if (value == Menu.info) {
      Get.dialog(InfoDialog(entity));
      return;
    }

    if (value == Menu.delete) {
      Repository.to.moveToTrash(entity.path);
      return;
    }

    if (value == Menu.deleteForever) {
      Repository.to.driveService.deleteByPath(entity.path);
      return;
    }

    if (value == Menu.rename) {
      Get.dialog(RenameEntityDialog(entity));
      return;
    }

    if (value == Menu.download) {
      if (entity is FileMetadata) {
        _downloadFile(entity);
      } else if (entity is FolderMetadata) {
        // TODO: Implement folder download (zip archive)
        toastification.show(
          context: Get.context!,
          type: ToastificationType.info,
          style: ToastificationStyle.fillColored,
          title: Text('Not implemented'),
          description: Text('Folder download is not yet implemented'),
          alignment: Alignment.bottomRight,
          autoCloseDuration: Duration(seconds: 3),
        );
      }
      return;
    }

    if (value == Menu.move) {
      if (entity is FolderMetadata) {
        Get.dialog(FolderSelectionDialogView(entity));
      }
      return;
    }

    if (value == Menu.changelog) {
      if (entity is FileMetadata) {
        Get.dialog(ChangelogDialogView(entity));
      }
      return;
    }

    if (value == Menu.restore) {
      Repository.to.driveService.move(
        oldPath: entity.path,
        newPath: p.join(myFilesPath, entity.name),
      );
      return;
    }

    if (value == Menu.sync) {
      String? selectedDirectory = await FilePicker.platform.getDirectoryPath();

      if (selectedDirectory == null) return;

      return;
    }

    if (value == Menu.share) {
      _shareEntity(entity);
      return;
    }
  }

  Future<void> _shareEntity(DriveItem entity) async {
    try {
      // Generate share link
      final shareLink = await Repository.to.generateShareLink(entity);
      
      if (shareLink != null) {
        // Copy to clipboard
        await Clipboard.setData(ClipboardData(text: shareLink));
        
        // Show success toast
        toastification.show(
          context: Get.context,
          type: ToastificationType.success,
          style: ToastificationStyle.fillColored,
          alignment: Alignment.bottomRight,
          title: Text("Link copied!"),
          description: Text("Share link has been copied to clipboard"),
          autoCloseDuration: Duration(seconds: 3),
        );
      } else {
        // Show error if link generation failed
        toastification.show(
          context: Get.context,
          type: ToastificationType.error,
          style: ToastificationStyle.fillColored,
          alignment: Alignment.bottomRight,
          title: Text("Share failed"),
          description: Text("Could not generate share link"),
          autoCloseDuration: Duration(seconds: 3),
        );
      }
    } catch (e) {
      toastification.show(
        context: Get.context,
        type: ToastificationType.error,
        style: ToastificationStyle.fillColored,
        alignment: Alignment.bottomRight,
        title: Text("Error"),
        description: Text("Failed to share: ${e.toString()}"),
        autoCloseDuration: Duration(seconds: 3),
      );
    }
  }

  Future<void> _downloadFile(FileMetadata file) async {
    try {
      // Get file data from the drive service
      // Pass hash and decryption parameters if the file is encrypted
      final fileData = await Repository.to.driveService.downloadFile(
        hash: file.hash,
        decryptionKey: file.decryptionKey,
        decryptionNonce: file.decryptionNonce,
      );

      // Save the file
      await FileSaver.instance.saveFile(
        name: file.name,
        bytes: fileData,
        mimeType: MimeType.other,
      );

      toastification.show(
        context: Get.context!,
        type: ToastificationType.success,
        style: ToastificationStyle.fillColored,
        title: Text('Download complete'),
        description: Text('${file.name} has been downloaded'),
        alignment: Alignment.bottomRight,
        autoCloseDuration: Duration(seconds: 3),
      );
    } catch (e) {
      // Error downloading file: $e
      toastification.show(
        context: Get.context!,
        type: ToastificationType.error,
        style: ToastificationStyle.fillColored,
        title: Text('Download failed'),
        description: Text('Failed to download ${file.name}'),
        alignment: Alignment.bottomRight,
        autoCloseDuration: Duration(seconds: 4),
      );
    }
  }
}
