import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/nostr_utils/nsec_to_hex.dart';
import 'package:lupine/repository.dart';

class LoginController extends GetxController {
  static LoginController get to => Get.find();

  final nsecController = TextEditingController();

  String get nsec => nsecController.text;

  void nsecChanged(String value) {
    if (!nsec.startsWith("nsec")) return;

    String privkey;
    try {
      privkey = nsecToHex(nsec);
    } catch (e) {
      return;
    }

    Repository.to.storage.write(key: "privkey", value: privkey);
    Repository.to.login(privkey);

    Get.offNamed(AppRoutes.home);
  }
}
