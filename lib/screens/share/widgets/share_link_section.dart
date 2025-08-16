import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/share/share_controller.dart';
import 'package:share_plus/share_plus.dart';
import 'package:toastification/toastification.dart';

class ShareLinkSection extends StatelessWidget {
  final ShareController controller;

  const ShareLinkSection({super.key, required this.controller});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Share via link', style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        Obx(() {
          if (controller.shareLink.value == null &&
              !controller.isGeneratingLink.value) {
            return Center(
              child: ElevatedButton.icon(
                onPressed: controller.generateShareLink,
                icon: const Icon(Icons.link),
                label: const Text('Generate Share Link'),
              ),
            );
          }

          if (controller.isGeneratingLink.value) {
            return const Center(child: CircularProgressIndicator());
          }

          if (controller.shareLink.value != null) {
            return _buildShareLinkDisplay(context);
          }

          return const SizedBox.shrink();
        }),
      ],
    );
  }

  Widget _buildShareLinkDisplay(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  controller.shareLink.value!,
                  style: Theme.of(context).textTheme.bodySmall,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              IconButton(
                icon: const Icon(Icons.copy, size: 20),
                onPressed: () => _copyLink(context),
                tooltip: 'Copy link',
              ),
              IconButton(
                icon: const Icon(Icons.share, size: 20),
                onPressed: _shareLink,
                tooltip: 'Share link',
              ),
            ],
          ),
          const SizedBox(height: 8),
          Text(
            'Anyone with this link can access the file',
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }

  void _copyLink(BuildContext context) {
    Clipboard.setData(ClipboardData(text: controller.shareLink.value!));
    toastification.show(
      context: context,
      type: ToastificationType.success,
      style: ToastificationStyle.fillColored,
      alignment: Alignment.bottomRight,
      title: const Text("Copied"),
      description: const Text("Link copied to clipboard"),
      autoCloseDuration: const Duration(seconds: 2),
    );
  }

  void _shareLink() {
    SharePlus.instance.share(ShareParams(
      uri: Uri.parse(controller.shareLink.value!)
    ));
  }
}
