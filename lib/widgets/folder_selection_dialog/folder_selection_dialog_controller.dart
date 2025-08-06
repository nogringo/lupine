import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:path/path.dart' as p;

class FolderSelectionDialogController extends GetxController {
  static FolderSelectionDialogController get to => Get.find();

  FolderMetadata entity;
  late String _folderPath;

  String get folderPath => _folderPath;
  set folderPath(String value) {
    _folderPath = value;
    update();
  }

  Future<List<FolderMetadata>> get folders async {
    final allItems = await Repository.to.driveService.list(folderPath);
    return allItems
        .whereType<FolderMetadata>()
        .where((e) => p.equals(folderPath, p.dirname(e.path)))
        .toList();
  }

  FolderSelectionDialogController(this.entity) {
    _folderPath = p.dirname(entity.path);
  }

  void goToParentDirectory() {
    folderPath = p.dirname(folderPath);
  }
}
