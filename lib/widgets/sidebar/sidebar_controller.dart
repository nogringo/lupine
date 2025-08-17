import 'package:get/get.dart';
import 'package:get_storage/get_storage.dart';
import 'package:lupine/config.dart';
import 'package:lupine/repository.dart';

class SidebarController extends GetxController {
  static SidebarController get to => Get.find();

  final _storage = GetStorage(appTitle);
  late bool _extended = _storage.read('navigationRailExtended') ?? true;

  bool get extended => _extended;
  set extended(bool value) {
    _extended = value;
    _storage.write('navigationRailExtended', value);
    update();
  }

  void toogleExtend() {
    extended = !extended;
  }

  void goTo(String path) {
    Repository.to.fileExplorerViewPath = path;
  }
}
