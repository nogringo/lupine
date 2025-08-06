import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:lupine/config.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/entity_thumbnail_view.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_context_menu.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart';
import 'package:lupine/utils/format_bytes.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

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

          return DataRow(
            cells: [
              DataCell(
                Row(
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
