import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/explorer/explorer_page.dart';
import 'package:lupine/screens/login/login_page.dart';

class LoadingController extends GetxController {
  static LoadingController get to => Get.find();

  loadApp() async {
    await Repository.to.storage.deleteAll();
    final privkey = await Repository.to.storage.read(key: "privkey");

    if (privkey == null) {
      Get.to(() => LoginPage());
      return;
    }

    Repository.to.initDriveService(privkey);
    Get.to(() => ExplorerPage());
  }
}