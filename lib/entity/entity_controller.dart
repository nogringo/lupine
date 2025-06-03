import 'package:get/get.dart';

class EntityController extends GetxController {
  static EntityController get to => Get.find();

  bool _isHovered = false;

  bool get isHovered => _isHovered;
  set isHovered(bool value) {
    _isHovered = value;
    update();
  }
}