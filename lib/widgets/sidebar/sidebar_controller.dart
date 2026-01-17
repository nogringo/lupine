import 'package:get/get.dart';
import 'package:lupine/local_storage_service.dart';
import 'package:lupine/repository.dart';

class SidebarController extends GetxController {
  static SidebarController get to => Get.find();

  late bool _extended =
      LocalStorageService.to.read('navigationRailExtended') ?? true;

  bool get extended => _extended;
  set extended(bool value) {
    _extended = value;
    LocalStorageService.to.write('navigationRailExtended', value);
    update();
  }

  void toogleExtend() {
    extended = !extended;
  }

  void goTo(String path) {
    Repository.to.fileExplorerViewPath = path;
  }
}
