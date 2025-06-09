import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_view.dart';
import 'package:lupine/widgets/folder_selection_dialog/folder_selection_dialog_view.dart';
import 'package:lupine/widgets/rename_entity_dialog.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

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

  List<DriveEvent> get entities {
    List<int Function(DriveEvent, DriveEvent)> sortByColumnAsc = [
      (a, b) => a.name.compareTo(b.name),
      (a, b) => a.createdAt.compareTo(b.createdAt),
      (a, b) => a.size.compareTo(b.size),
    ];

    List<int Function(DriveEvent, DriveEvent)> sortByColumnDec = [
      (a, b) => b.name.compareTo(a.name),
      (a, b) => b.createdAt.compareTo(a.createdAt),
      (a, b) => b.size.compareTo(a.size),
    ];

    Repository.to.driveEvents.sort(
      sortAscending
          ? sortByColumnAsc[sortColumnIndex]
          : sortByColumnDec[sortColumnIndex],
    );
    return Repository.to.driveEvents;
  }

  void onSort(int columnIndex, bool ascending) {
    sortColumnIndex = columnIndex;
    sortAscending = ascending;
  }

  void onEntityMenuSelected(Menu value, DriveEvent entity) {
    if (value == Menu.delete) {
      Repository.to.deleteEntity(entity);
      return;
    }

    if (value == Menu.emptyTrash) {
      Repository.to.emptyTrash();
      return;
    }

    if (value == Menu.rename) {
      Get.dialog(RenameEntityDialog(entity));
      return;
    }

    if (value == Menu.download) {
      DriveService().downloadEntity(entity);
      return;
    }

    if (value == Menu.move) {
      Get.dialog(FolderSelectionDialogView(entity));
      return;
    }
  }
}
