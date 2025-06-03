import 'package:get/get.dart';
import 'package:lupine/repository.dart';

class SidebarController extends GetxController {
  static SidebarController get to => Get.find();

  bool _extended = true;

  bool get extended => _extended;
  set extended(bool value) {
    _extended = value;
    update();
  }

  void toogleExtend() {
    extended = !extended;
  }

  void goTo(String path) {
    Repository.to.fileExplorerViewPath = path;
  }
}