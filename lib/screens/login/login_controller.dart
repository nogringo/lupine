import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/repository.dart';
import 'package:toastification/toastification.dart';

class LoginController extends GetxController {
  static LoginController get to => Get.find();

  final RxBool isLoading = false.obs;

  Future<void> onLoggedIn() async {
    isLoading.value = true;

    try {
      await Repository.to.onLogin();

      Get.offAllNamed(AppRoutes.home);
    } catch (e) {
      // Handle login error
      toastification.show(
        context: Get.context,
        type: ToastificationType.error,
        style: ToastificationStyle.fillColored,
        alignment: Alignment.bottomRight,
        title: Text("Login Error"),
        description: Text("Failed to complete login: ${e.toString()}"),
      );
    } finally {
      isLoading.value = false;
    }
  }
}
