import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/config.dart';
import 'package:lupine/screens/share/share_controller.dart';
import 'package:lupine/utils/format_bytes.dart';

class SharePage extends StatelessWidget {
  const SharePage({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text("$appTitle download"),
        actions: [
          IconButton(
            onPressed: () {
              Get.offAllNamed(AppRoutes.home);
            },
            icon: Icon(Icons.home),
          ),
          SizedBox(width: 8),
        ],
      ),
      body: GetBuilder(
        init: ShareController(),
        builder: (c) {
          if (c.isLocked) {
            return PasswordView();
          }

          if (c.isLoading) {
            return Align(
              alignment: Alignment(0, -0.3),
              child: Text("Loading file ..."),
            );
          }

          return DownloadView();
        },
      ),
    );
  }
}

class PasswordView extends StatelessWidget {
  const PasswordView({super.key});

  @override
  Widget build(BuildContext context) {
    final c = ShareController.to;
    return Align(
      alignment: Alignment(0, -0.3),
      child: Container(
        padding: const EdgeInsets.all(24),
        constraints: const BoxConstraints(maxWidth: 400),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'Password Protected',
              style: Theme.of(context).textTheme.headlineSmall,
            ),
            const SizedBox(height: 8),
            Text(
              'Enter the password to access this shared file',
              style: Theme.of(context).textTheme.bodyMedium,
            ),
            const SizedBox(height: 32),
            TextField(
              controller: c.passwordController,
              obscureText: true,
              autofocus: true,
              decoration: const InputDecoration(labelText: 'Password'),
              onSubmitted: (_) => c.unlock(),
            ),
            const SizedBox(height: 24),
            FilledButton(onPressed: c.unlock, child: const Text('Unlock')),
          ],
        ),
      ),
    );
  }
}

class DownloadView extends StatelessWidget {
  const DownloadView({super.key});

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment(0, -0.3),
      child: Container(
        padding: EdgeInsets.all(16),
        constraints: const BoxConstraints(maxWidth: 400),
        decoration: BoxDecoration(
          border: Border.all(
            color: Theme.of(context).colorScheme.outline.withValues(alpha: 0.6),
            width: 2,
          ),
          borderRadius: BorderRadius.circular(32),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              ShareController.to.driveItem!.fileName,
              style: Theme.of(context).textTheme.titleLarge,
            ),
            Text(
              formatBytes(ShareController.to.driveItem!.size),
              style: Theme.of(context).textTheme.labelLarge,
            ),
            SizedBox(height: 16),
            Obx(
              () => FilledButton(
                onPressed: ShareController.to.isDownloading.value
                    ? null
                    : ShareController.to.downloadFile,
                child: Text("Download"),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
