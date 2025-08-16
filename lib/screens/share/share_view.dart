import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/share/share_controller.dart';
import 'package:lupine/screens/share/widgets/share_link_section.dart';
import 'package:lupine/screens/share/widgets/user_search_section.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class ShareView extends StatelessWidget {
  final DriveItem entity;

  const ShareView({super.key, required this.entity});

  @override
  Widget build(BuildContext context) {
    final isSmallScreen = MediaQuery.of(context).size.width < 600;

    return GetBuilder(
      init: ShareController(entity: entity),
      builder: (controller) {
        if (isSmallScreen) {
          return _buildFullScreenDialog(context, controller);
        } else {
          return _buildDialog(context, controller);
        }
      },
    );
  }

  Widget _buildFullScreenDialog(
    BuildContext context,
    ShareController controller,
  ) {
    return Scaffold(
      appBar: AppBar(
        title: Text('Share "${entity.name}"'),
        leading: IconButton(
          icon: const Icon(Icons.close),
          onPressed: () => Navigator.of(context).pop(),
        ),
        actions: [
          Obx(
            () => TextButton(
              onPressed: controller.canShare.value
                  ? controller.shareWithUser
                  : null,
              child: const Text('Share'),
            ),
          ),
        ],
      ),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            UserSearchSection(controller: controller),
            const SizedBox(height: 24),
            ShareLinkSection(controller: controller),
          ],
        ),
      ),
    );
  }

  Widget _buildDialog(BuildContext context, ShareController controller) {
    return Dialog(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 600, maxHeight: 700),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.all(16.0),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      'Share "${entity.name}"',
                      style: Theme.of(context).textTheme.headlineSmall,
                    ),
                  ),
                  IconButton(
                    icon: const Icon(Icons.close),
                    onPressed: () => Navigator.of(context).pop(),
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
            Flexible(
              child: SingleChildScrollView(
                padding: const EdgeInsets.all(16.0),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    UserSearchSection(controller: controller),
                    const SizedBox(height: 24),
                    ShareLinkSection(controller: controller),
                  ],
                ),
              ),
            ),
            const Divider(height: 1),
            Padding(
              padding: const EdgeInsets.all(16.0),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  TextButton(
                    onPressed: () => Navigator.of(context).pop(),
                    child: const Text('Cancel'),
                  ),
                  const SizedBox(width: 8),
                  Obx(
                    () => FilledButton(
                      onPressed: controller.canShare.value
                          ? controller.shareWithUser
                          : null,
                      child: const Text('Share with user'),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
