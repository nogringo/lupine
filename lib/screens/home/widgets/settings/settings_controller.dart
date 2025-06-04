import 'package:get/get.dart';
import 'package:lupine/repository.dart';

class SettingsController extends GetxController {
  static SettingsController get to => Get.find();
  

  void logout() async {
    Repository.to.logout();
  }
}