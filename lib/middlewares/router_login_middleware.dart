import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';

import '../app_routes.dart';

class RouterLoginMiddleware extends GetMiddleware {
  @override
  RouteSettings? redirect(String? route) {
    if (Repository.to.driveService.ndk.accounts.isNotLoggedIn) {
      return const RouteSettings(name: AppRoutes.login);
    }
    return null;
  }
}
