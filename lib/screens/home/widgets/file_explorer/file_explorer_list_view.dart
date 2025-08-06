import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:lupine/config.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/entity_thumbnail_view.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_context_menu.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart';
import 'package:lupine/utils/format_bytes.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:path/path.dart' as p;

class FileExplorerListView extends StatelessWidget {
  final List<DriveItem> entities;
  final FileExplorerController controller;
  final BoxConstraints constraints;

  const FileExplorerListView({
    super.key,
    required this.entities,
    required this.controller,
    required this.constraints,
  });

  @override
  Widget build(BuildContext context) {
    final width = constraints.maxWidth;
    final showModified = width > 800;
    final showSize = width > 600;

    return SingleChildScrollView(
      scrollDirection: Axis.vertical,
      child: DataTable(
        showCheckboxColumn: false,
        sortColumnIndex: controller.sortColumnIndex,
        sortAscending: controller.sortAscending,
        columns: [
          DataColumn(
            label: Text("Name"),
            columnWidth: FlexColumnWidth(),
            onSort: controller.onSort,
          ),
          if (showModified)
            DataColumn(label: Text("Modified"), onSort: controller.onSort),
          if (showSize)
            DataColumn(label: Text("Size"), onSort: controller.onSort),
          DataColumn(label: Container()),
        ],
        rows: List.generate(entities.length, (index) {
          final entity = entities[index];
          final isFolder = entity is FolderMetadata;

          return DataRow(
            cells: [
              DataCell(
                Draggable<DriveItem>(
                  data: entity,
                  feedback: Material(
                    elevation: 4,
                    borderRadius: BorderRadius.circular(8),
                    child: Container(
                      padding: EdgeInsets.symmetric(horizontal: 16, vertical: 8),
                      decoration: BoxDecoration(
                        color: Theme.of(context).colorScheme.surfaceContainer,
                        borderRadius: BorderRadius.circular(8),
                      ),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          SizedBox(
                            width: 16,
                            height: 16,
                            child: EntityThumbnailView(entity),
                          ),
                          SizedBox(width: 8),
                          Text(
                            entity.name,
                            style: TextStyle(
                              color: Theme.of(context).colorScheme.onSurface,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                  child: isFolder
                      ? DragTarget<DriveItem>(
                          onWillAcceptWithDetails: (details) {
                            final draggedItem = details.data;
                            if (draggedItem.path == entity.path) return false;
                            if (entity.path.startsWith('${draggedItem.path}/')) return false;
                            return true;
                          },
                          onAcceptWithDetails: (details) {
                            final draggedItem = details.data;
                            final newPath = p.join(entity.path, draggedItem.name);
                            Repository.to.driveService.move(
                              oldPath: draggedItem.path,
                              newPath: newPath,
                            );
                          },
                          builder: (context, candidateData, rejectedData) {
                            final isHovering = candidateData.isNotEmpty;
                            return Container(
                              padding: isHovering ? EdgeInsets.all(2) : null,
                              decoration: BoxDecoration(
                                color: isHovering
                                    ? Theme.of(context).colorScheme.primaryContainer.withValues(alpha: 0.2)
                                    : null,
                                borderRadius: BorderRadius.circular(4),
                              ),
                              child: Row(
                                children: [
                                  SizedBox(
                                    width: 16,
                                    height: 16,
                                    child: EntityThumbnailView(entity),
                                  ),
                                  SizedBox(width: 8),
                                  Expanded(
                                    child: Text(entity.name, overflow: TextOverflow.ellipsis),
                                  ),
                                ],
                              ),
                            );
                          },
                        )
                      : Row(
                          children: [
                            SizedBox(
                              width: 16,
                              height: 16,
                              child: EntityThumbnailView(entity),
                            ),
                            SizedBox(width: 8),
                            Expanded(
                              child: Text(entity.name, overflow: TextOverflow.ellipsis),
                            ),
                          ],
                        ),
                ),
                onTap: () {
                  if (entity is FolderMetadata) {
                    Repository.to.fileExplorerViewPath = entity.path;
                  }
                },
              ),
              if (showModified)
                DataCell(Text(DateFormat().format(entity.createdAt))),
              if (showSize)
                DataCell(
                  Text(entity is FileMetadata ? formatBytes(entity.size) : ''),
                ),
              DataCell(
                PopupMenuButton(
                  shape: RoundedRectangleBorder(
                    borderRadius: BorderRadius.circular(12),
                    side: BorderSide(
                      color: Theme.of(context).dividerColor,
                      width: 1,
                    ),
                  ),
                  itemBuilder: (context) {
                    final isInTrashcan = entity.parentPath == trashPath;
                    return FileExplorerContextMenu.buildMenuItems(
                      context,
                      entity,
                      isInTrashcan,
                    );
                  },
                  onSelected:
                      (value) => controller.onEntityMenuSelected(value, entity),
                ),
              ),
            ],
            onSelectChanged: (value) {},
          );
        }),
      ),
    );
  }
}
