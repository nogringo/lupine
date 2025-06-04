import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/home_page.dart';
import 'package:lupine/screens/login/login_page.dart';

class LoadingController extends GetxController {
  static LoadingController get to => Get.find();

  loadApp() async {
    final privkey = await Repository.to.storage.read(key: "privkey");

    if (privkey == null) {
      Get.off(() => LoginPage());
      return;
    }

    Repository.to.initDriveService(privkey);
    Get.off(() => HomePage());
  }
}