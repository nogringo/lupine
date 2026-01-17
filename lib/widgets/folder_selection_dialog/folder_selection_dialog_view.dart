import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/widgets/folder_selection_dialog/folder_selection_dialog_controller.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:path/path.dart' as p;

class FolderSelectionDialogView extends StatelessWidget {
  final FolderMetadata entity;

  const FolderSelectionDialogView(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder(
      init: FolderSelectionDialogController(entity),
      builder: (FolderSelectionDialogController c) {
        return Dialog(
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(16),
          ),
          child: SizedBox(
            width: 500,
            height: 600,
            child: Column(
              children: [
                // Header
                Container(
                  padding: EdgeInsets.all(20),
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.primaryContainer,
                    borderRadius: BorderRadius.only(
                      topLeft: Radius.circular(16),
                      topRight: Radius.circular(16),
                    ),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Text(
                            'Move "${entity.name}"',
                            style: Theme.of(context).textTheme.titleLarge
                                ?.copyWith(
                                  color: Theme.of(
                                    context,
                                  ).colorScheme.onPrimaryContainer,
                                  fontWeight: FontWeight.w600,
                                ),
                          ),
                          IconButton(
                            icon: Icon(Icons.close),
                            onPressed: () => Get.back(),
                            color: Theme.of(
                              context,
                            ).colorScheme.onPrimaryContainer,
                          ),
                        ],
                      ),
                      SizedBox(height: 8),
                      Text(
                        'Select destination folder',
                        style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                          color: Theme.of(context)
                              .colorScheme
                              .onPrimaryContainer
                              .withValues(alpha: 0.8),
                        ),
                      ),
                    ],
                  ),
                ),

                // Breadcrumb navigation
                Container(
                  padding: EdgeInsets.symmetric(horizontal: 20, vertical: 12),
                  decoration: BoxDecoration(
                    color: Theme.of(
                      context,
                    ).colorScheme.surfaceContainerHighest,
                    border: Border(
                      bottom: BorderSide(
                        color: Theme.of(context).dividerColor,
                        width: 1,
                      ),
                    ),
                  ),
                  child: Row(
                    children: [
                      IconButton(
                        icon: Icon(Icons.arrow_upward),
                        onPressed: c.folderPath == '/'
                            ? null
                            : () {
                                c.goToParentDirectory();
                              },
                        tooltip: 'Go to parent folder',
                      ),
                      SizedBox(width: 8),
                      Icon(Icons.folder, size: 20),
                      SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          c.folderPath == '/' ? 'Root' : c.folderPath,
                          style: Theme.of(context).textTheme.bodyMedium,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                    ],
                  ),
                ),

                // Folder list
                Expanded(
                  child: FutureBuilder<List<FolderMetadata>>(
                    future: c.folders,
                    builder: (context, snapshot) {
                      if (!snapshot.hasData) {
                        return Center(child: CircularProgressIndicator());
                      }

                      final folders = snapshot.data!;

                      if (folders.isEmpty) {
                        return Center(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Icon(
                                Icons.folder_off,
                                size: 64,
                                color: Theme.of(context).colorScheme.outline,
                              ),
                              SizedBox(height: 16),
                              Text(
                                'No folders here',
                                style: Theme.of(context).textTheme.bodyLarge
                                    ?.copyWith(
                                      color: Theme.of(
                                        context,
                                      ).colorScheme.outline,
                                    ),
                              ),
                              SizedBox(height: 8),
                              Text(
                                'You can move the item to the current location\nor go back to parent folder',
                                style: Theme.of(context).textTheme.bodySmall
                                    ?.copyWith(
                                      color: Theme.of(
                                        context,
                                      ).colorScheme.outline,
                                    ),
                                textAlign: TextAlign.center,
                              ),
                            ],
                          ),
                        );
                      }

                      return ListView.builder(
                        padding: EdgeInsets.symmetric(vertical: 8),
                        itemCount: folders.length,
                        itemBuilder: (context, index) {
                          final folder = folders[index];
                          final isSelected = folder.path == c.folderPath;

                          return ListTile(
                            leading: Icon(
                              Icons.folder,
                              color: Theme.of(context).colorScheme.primary,
                            ),
                            title: Text(
                              folder.name,
                              style: TextStyle(
                                fontWeight: isSelected
                                    ? FontWeight.w600
                                    : FontWeight.normal,
                              ),
                            ),
                            subtitle: Text(
                              'Click to open folder',
                              style: Theme.of(context).textTheme.bodySmall,
                            ),
                            selected: isSelected,
                            onTap: () {
                              c.folderPath = folder.path;
                            },
                            trailing: IconButton(
                              icon: Icon(Icons.arrow_forward_ios, size: 16),
                              onPressed: () {
                                c.folderPath = folder.path;
                              },
                            ),
                          );
                        },
                      );
                    },
                  ),
                ),

                // Actions
                Container(
                  padding: EdgeInsets.all(20),
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.surfaceContainerHigh,
                    borderRadius: BorderRadius.only(
                      bottomLeft: Radius.circular(16),
                      bottomRight: Radius.circular(16),
                    ),
                    border: Border(
                      top: BorderSide(
                        color: Theme.of(context).dividerColor,
                        width: 1,
                      ),
                    ),
                  ),
                  child: Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      Text(
                        'Move to: ${c.folderPath == '/' ? 'Root' : p.basename(c.folderPath)}',
                        style: Theme.of(context).textTheme.bodySmall,
                      ),
                      Row(
                        children: [
                          TextButton(
                            onPressed: () => Get.back(),
                            child: Text('Cancel'),
                          ),
                          SizedBox(width: 8),
                          FilledButton.icon(
                            onPressed: () {
                              final newPath = p.join(c.folderPath, entity.name);
                              Repository.to.driveService.move(
                                oldPath: entity.path,
                                newPath: newPath,
                              );
                              Get.back();
                            },
                            icon: Icon(Icons.drive_file_move),
                            label: Text('Move Here'),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}
