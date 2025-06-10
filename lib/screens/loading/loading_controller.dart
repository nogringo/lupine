import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/repository.dart';

class LoadingController extends GetxController {
  static LoadingController get to => Get.find();

  loadApp() async {
    await Repository.to.init();

    final privkey = await Repository.to.storage.read(key: "privkey");

    if (privkey == null) {
      Get.offNamed(AppRoutes.login);
      return;
    }

    Repository.to.login(privkey);
    Get.offNamed(AppRoutes.home);
  }
}