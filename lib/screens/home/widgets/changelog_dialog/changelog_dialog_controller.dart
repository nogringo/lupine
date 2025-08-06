import 'package:get/get.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

enum Action { delete, restore }

class ChangelogDialogController extends GetxController {
  static ChangelogDialogController get to => Get.find();

  FileMetadata? driveEvent;
  Action action = Action.delete;

  void restoreVersion(FileMetadata e) {
    driveEvent = e;
    action = Action.restore;
    update();
  }

  void deleteVersion(FileMetadata e) {
    driveEvent = e;
    action = Action.delete;
    update();
  }

  void process() {}
}
