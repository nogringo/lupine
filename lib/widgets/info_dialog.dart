import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:intl/intl.dart';
import 'package:lupine/utils/format_bytes.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:toastification/toastification.dart';

class InfoDialog extends StatelessWidget {
  final DriveItem entity;

  const InfoDialog(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    final isFile = entity is FileMetadata;
    final file = isFile ? entity as FileMetadata : null;

    return AlertDialog(
      title: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Expanded(
            child: Row(
              children: [
                Icon(isFile ? Icons.insert_drive_file : Icons.folder, size: 24),
                SizedBox(width: 8),
                Expanded(
                  child: Text(entity.name, overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
          ),
          CloseButton(),
        ],
      ),
      content: SizedBox(
        width: 400,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _buildInfoRow('Type', isFile ? 'File' : 'Folder'),
            _buildInfoRow('Name', entity.name),
            _buildInfoRow('Path', entity.path, copyable: true),
            if (isFile) ...[
              _buildInfoRow('Size', formatBytes(file!.size)),
              if (file.fileType != null)
                _buildInfoRow('MIME Type', file.fileType!),
              if (file.fileExtension.isNotEmpty)
                _buildInfoRow('Extension', '.${file.fileExtension}'),
              _buildInfoRow(
                'Encrypted',
                file.isEncrypted ? 'Yes' : 'No',
                valueColor: file.isEncrypted ? Colors.green : null,
              ),
              if (file.isEncrypted && file.encryptionAlgorithm != null)
                _buildInfoRow('Encryption', file.encryptionAlgorithm!),
              _buildInfoRow('Hash', file.hash, copyable: true, monospace: true),
            ],
            _buildInfoRow(
              'Created',
              DateFormat('MMM dd, yyyy HH:mm').format(entity.createdAt),
            ),
            if (entity.eventId != null)
              _buildInfoRow(
                'Event ID',
                entity.eventId!,
                copyable: true,
                monospace: true,
              ),
          ],
        ),
      ),
      actions: [TextButton(onPressed: () => Get.back(), child: Text('Close'))],
    );
  }

  Widget _buildInfoRow(
    String label,
    String value, {
    bool copyable = false,
    bool monospace = false,
    Color? valueColor,
  }) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 120,
            child: Text(
              label,
              style: TextStyle(
                fontWeight: FontWeight.w500,
                color: Colors.grey[600],
              ),
            ),
          ),
          Expanded(
            child: Row(
              children: [
                Expanded(
                  child: SelectableText(
                    value,
                    style: TextStyle(
                      fontFamily: monospace ? 'monospace' : null,
                      fontSize: monospace ? 12 : null,
                      color: valueColor,
                    ),
                  ),
                ),
                if (copyable)
                  IconButton(
                    icon: Icon(Icons.copy, size: 16),
                    padding: EdgeInsets.zero,
                    constraints: BoxConstraints(minWidth: 32, minHeight: 32),
                    onPressed: () {
                      Clipboard.setData(ClipboardData(text: value));
                      toastification.show(
                        context: Get.context!,
                        type: ToastificationType.success,
                        style: ToastificationStyle.fillColored,
                        title: Text('Copied to clipboard'),
                        alignment: Alignment.bottomRight,
                        autoCloseDuration: Duration(seconds: 2),
                      );
                    },
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
