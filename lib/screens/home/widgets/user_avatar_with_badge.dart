import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/controllers/server_status_controller.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/widgets/user_dialog.dart';
import 'package:nostr_widgets/widgets/widgets.dart';

class UserAvatarWithBadge extends StatelessWidget {
  const UserAvatarWithBadge({super.key});

  @override
  Widget build(BuildContext context) {
    // Initialize controller if not already initialized
    if (!Get.isRegistered<ServerStatusController>()) {
      Get.put(ServerStatusController());
    }

    return GetBuilder<ServerStatusController>(
      builder: (controller) {
        final showBadge = controller.needsConfiguration;
        final hasNoBlossomServers = controller.hasNoBlossomServers;

        // Use error color if no blossom servers, otherwise use amber for warning
        final badgeColor = hasNoBlossomServers
            ? Theme.of(context).colorScheme.error
            : Theme.of(context).colorScheme.primary;

        // Different message based on severity
        final tooltipMessage = hasNoBlossomServers
            ? 'Critical: No storage servers configured!'
            : 'Configuration needed: Add more relays or storage servers';

        Widget avatarWidget = showBadge
            ? Badge(
                smallSize: 12,
                backgroundColor: badgeColor,
                child: NPicture(ndk: Repository.to.ndk),
              )
            : NPicture(ndk: Repository.to.ndk);

        // Show user dialog popup when clicked
        Widget clickableAvatar = GestureDetector(
          onTap: () {
            showUserDialog(context);
          },
          child: avatarWidget,
        );

        if (showBadge) {
          return Tooltip(message: tooltipMessage, child: clickableAvatar);
        }

        return clickableAvatar;
      },
    );
  }
}
