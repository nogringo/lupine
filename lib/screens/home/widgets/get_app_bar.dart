import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/screens/home/widgets/profile_picture_view.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:window_manager/window_manager.dart';

AppBar getAppBar() {
  return AppBar(
    backgroundColor: Colors.transparent,
    surfaceTintColor: Colors.transparent,
    title: () {
      final title = Align(
        alignment: AlignmentDirectional.centerStart,
        child: Text(appTitle),
      );
      if (isDesktop) return DragToMoveArea(child: title);
      return title;
    }(),
    actions: [
      if (kIsWeb || GetPlatform.isMobile) ProfilePictureView(),
      if (isDesktop)
        Align(alignment: Alignment.topCenter, child: WindowButtons()),
      SizedBox(width: 8),
    ],
  );
}
