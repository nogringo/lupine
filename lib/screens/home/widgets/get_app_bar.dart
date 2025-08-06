import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:nostr_widgets/widgets/widgets.dart';
import 'package:window_manager/window_manager.dart';
import 'package:flutter_svg/flutter_svg.dart';

AppBar getAppBar() {
  return AppBar(
    backgroundColor: Colors.transparent,
    surfaceTintColor: Colors.transparent,
    title: () {
      final title = Align(
        alignment: AlignmentDirectional.centerStart,
        child: Row(
          children: [
            SvgPicture.asset(
              "images/lupinepath.svg",
              colorFilter: ColorFilter.mode(
                Theme.of(Get.context!).colorScheme.primary,
                BlendMode.srcIn,
              ),
              semanticsLabel: 'Lupine logo',
              height: kToolbarHeight * .8,
            ),
            Text(appTitle),
          ],
        ),
      );
      if (isDesktop) return DragToMoveArea(child: title);
      return title;
    }(),
    actions: [
      NPicture(ndk: Repository.to.ndk),
      SizedBox(width: 8),
      if (isDesktop)
        Align(alignment: Alignment.topCenter, child: WindowButtons()),
    ],
  );
}
