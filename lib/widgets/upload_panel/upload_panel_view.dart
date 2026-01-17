import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/controllers/upload_queue_controller.dart';
import 'package:lupine/models/upload_item.dart';
import 'package:lupine/utils/format_bytes.dart';

class UploadPanelView extends StatelessWidget {
  const UploadPanelView({super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder<UploadQueueController>(
      builder: (controller) {
        if (!controller.hasItems) {
          return const SizedBox.shrink();
        }

        return Positioned(
          right: 16,
          bottom: 16,
          child: Material(
            elevation: 8,
            borderRadius: BorderRadius.circular(12),
            clipBehavior: Clip.antiAlias,
            child: Container(
              width: 320,
              constraints: const BoxConstraints(maxHeight: 400),
              decoration: BoxDecoration(
                color: Theme.of(context).colorScheme.surfaceContainer,
                borderRadius: BorderRadius.circular(12),
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  _buildHeader(context, controller),
                  if (controller.isExpanded)
                    Flexible(child: _buildItemsList(context, controller)),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _buildHeader(BuildContext context, UploadQueueController controller) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    String headerText;
    if (controller.isAllCompleted) {
      final failedCount = controller.failedCount;
      if (failedCount > 0) {
        headerText =
            '${controller.completedCount} uploaded, $failedCount failed';
      } else {
        headerText = '${controller.completedCount} uploads complete';
      }
    } else {
      headerText =
          'Uploading ${controller.completedCount + 1} of ${controller.totalCount}';
    }

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      decoration: BoxDecoration(
        color: colorScheme.primaryContainer,
        borderRadius: controller.isExpanded
            ? const BorderRadius.vertical(top: Radius.circular(12))
            : BorderRadius.circular(12),
      ),
      child: Row(
        children: [
          if (!controller.isAllCompleted)
            Padding(
              padding: const EdgeInsets.only(right: 12),
              child: SizedBox(
                width: 18,
                height: 18,
                child: CircularProgressIndicator(
                  strokeWidth: 2,
                  color: colorScheme.onPrimaryContainer,
                ),
              ),
            ),
          if (controller.isAllCompleted)
            Padding(
              padding: const EdgeInsets.only(right: 12),
              child: Icon(
                controller.failedCount > 0
                    ? Icons.warning_rounded
                    : Icons.check_circle_rounded,
                size: 20,
                color: colorScheme.onPrimaryContainer,
              ),
            ),
          Expanded(
            child: Text(
              headerText,
              style: theme.textTheme.titleSmall?.copyWith(
                color: colorScheme.onPrimaryContainer,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          if (controller.isAllCompleted)
            IconButton(
              icon: Icon(
                Icons.close,
                size: 20,
                color: colorScheme.onPrimaryContainer,
              ),
              onPressed: controller.clearAll,
              padding: EdgeInsets.zero,
              constraints: const BoxConstraints(minWidth: 32, minHeight: 32),
              tooltip: 'Close',
            ),
          IconButton(
            icon: Icon(
              controller.isExpanded
                  ? Icons.keyboard_arrow_down
                  : Icons.keyboard_arrow_up,
              size: 20,
              color: colorScheme.onPrimaryContainer,
            ),
            onPressed: () => controller.isExpanded = !controller.isExpanded,
            padding: EdgeInsets.zero,
            constraints: const BoxConstraints(minWidth: 32, minHeight: 32),
            tooltip: controller.isExpanded ? 'Collapse' : 'Expand',
          ),
        ],
      ),
    );
  }

  Widget _buildItemsList(
    BuildContext context,
    UploadQueueController controller,
  ) {
    return ListView.builder(
      shrinkWrap: true,
      padding: const EdgeInsets.symmetric(vertical: 8),
      itemCount: controller.queue.length,
      itemBuilder: (context, index) {
        final item = controller.queue[index];
        return _UploadItemTile(item: item);
      },
    );
  }
}

class _UploadItemTile extends StatelessWidget {
  final UploadItem item;

  const _UploadItemTile({required this.item});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      child: Row(
        children: [
          _buildStatusIcon(colorScheme),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  item.fileName,
                  style: theme.textTheme.bodyMedium,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                const SizedBox(height: 2),
                Text(
                  _getStatusText(),
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: _getStatusColor(colorScheme),
                  ),
                ),
              ],
            ),
          ),
          Text(
            formatBytes(item.fileSize),
            style: theme.textTheme.bodySmall?.copyWith(
              color: colorScheme.onSurfaceVariant,
            ),
          ),
          if (item.status == UploadStatus.failed)
            IconButton(
              icon: Icon(Icons.refresh, size: 18, color: colorScheme.primary),
              onPressed: () => UploadQueueController.to.retry(item.id),
              padding: const EdgeInsets.all(4),
              constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
              tooltip: 'Retry',
            ),
        ],
      ),
    );
  }

  Widget _buildStatusIcon(ColorScheme colorScheme) {
    switch (item.status) {
      case UploadStatus.waiting:
        return Icon(
          Icons.schedule,
          size: 20,
          color: colorScheme.onSurfaceVariant,
        );
      case UploadStatus.uploading:
        return SizedBox(
          width: 20,
          height: 20,
          child: CircularProgressIndicator(
            strokeWidth: 2,
            color: colorScheme.primary,
          ),
        );
      case UploadStatus.completed:
        return Icon(Icons.check_circle, size: 20, color: colorScheme.primary);
      case UploadStatus.failed:
        return Icon(Icons.error, size: 20, color: colorScheme.error);
    }
  }

  String _getStatusText() {
    switch (item.status) {
      case UploadStatus.waiting:
        return 'Waiting...';
      case UploadStatus.uploading:
        return 'Uploading...';
      case UploadStatus.completed:
        return 'Complete';
      case UploadStatus.failed:
        return item.errorMessage ?? 'Failed';
    }
  }

  Color _getStatusColor(ColorScheme colorScheme) {
    switch (item.status) {
      case UploadStatus.waiting:
        return colorScheme.onSurfaceVariant;
      case UploadStatus.uploading:
        return colorScheme.primary;
      case UploadStatus.completed:
        return colorScheme.primary;
      case UploadStatus.failed:
        return colorScheme.error;
    }
  }
}
