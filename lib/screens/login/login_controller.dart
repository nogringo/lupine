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

      // Check if user needs onboarding
      final needsOnboarding = await _checkIfNeedsOnboarding();

      if (needsOnboarding) {
        Get.offAllNamed(AppRoutes.onboarding);
      } else {
        Get.offAllNamed(AppRoutes.home);
      }
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

  Future<bool> _checkIfNeedsOnboarding() async {
    try {
      final pubkey = Repository.to.ndk.accounts.getPublicKey();
      if (pubkey == null) return false;

      // Check relays count
      final userRelayLists = await Repository.to.ndk.userRelayLists
          .getSingleUserRelayList(pubkey);
      final relayCount = userRelayLists?.relays.keys.length ?? 0;

      // Check blossom servers count
      final blossomUserServerList = await Repository
          .to
          .driveService
          .ndk
          .blossomUserServerList
          .getUserServerList(pubkeys: [pubkey]);
      final blossomCount = blossomUserServerList?.length ?? 0;

      // Need onboarding if less than 2 relays OR less than 2 blossom servers
      return relayCount < 2 || blossomCount < 2;
    } catch (e) {
      // If there's an error checking, default to showing onboarding
      return true;
    }
  }
}
