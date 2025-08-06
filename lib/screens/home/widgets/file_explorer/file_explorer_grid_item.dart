import 'package:flutter/material.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_context_menu.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class FileExplorerGridItem extends StatelessWidget {
  final DriveItem entity;
  final FileExplorerController controller;

  const FileExplorerGridItem({
    super.key,
    required this.entity,
    required this.controller,
  });

  @override
  Widget build(BuildContext context) {
    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(24),
        side: BorderSide(
          color: Theme.of(context).dividerColor.withValues(alpha: 0.8),
          width: 2,
        ),
      ),
      child: InkWell(
        onTap: () {
          if (entity is FolderMetadata) {
            Repository.to.fileExplorerViewPath = entity.path;
          }
        },
        onSecondaryTapUp: (details) {
          _showContextMenu(context, details);
        },
        borderRadius: BorderRadius.circular(22),
        child: Stack(
          fit: StackFit.expand,
          children: [
            Column(
              children: [
                Expanded(
                  child: Icon(
                    (entity is FileMetadata)
                        ? Icons.insert_drive_file
                        : Icons.folder,
                    size: 64,
                    color: Theme.of(context).colorScheme.primary,
                  ),
                ),
                Padding(
                  padding: const EdgeInsets.all(12),
                  child: Text(entity.name, overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
            Positioned(
              top: 8,
              right: 8,
              child: IconButton(
                onPressed: () {
                  final RenderBox button =
                      context.findRenderObject() as RenderBox;
                  final RenderBox overlay =
                      Overlay.of(context).context.findRenderObject()
                          as RenderBox;
                  final position = RelativeRect.fromRect(
                    Rect.fromPoints(
                      button.localToGlobal(
                        Offset(button.size.width - 8, 8),
                        ancestor: overlay,
                      ),
                      button.localToGlobal(
                        Offset(button.size.width, 40),
                        ancestor: overlay,
                      ),
                    ),
                    Offset.zero & overlay.size,
                  );
                  FileExplorerContextMenu.showAtPosition(
                    context: context,
                    position: position,
                    entity: entity,
                    onMenuSelected: controller.onEntityMenuSelected,
                  );
                },
                icon: Icon(Icons.more_vert),
              ),
            ),
          ],
        ),
      ),
    );
  }

  void _showContextMenu(BuildContext context, TapUpDetails details) {
    final position = RelativeRect.fromLTRB(
      details.globalPosition.dx,
      details.globalPosition.dy,
      details.globalPosition.dx,
      details.globalPosition.dy,
    );

    FileExplorerContextMenu.showAtPosition(
      context: context,
      position: position,
      entity: entity,
      onMenuSelected: controller.onEntityMenuSelected,
    );
  }
}
