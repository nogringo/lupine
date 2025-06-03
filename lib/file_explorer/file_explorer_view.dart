import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:intl/intl.dart';
import 'package:lupine/config.dart';
import 'package:lupine/file_explorer/file_explorer_controller.dart';
import 'package:lupine/format_bytes.dart';
import 'package:lupine/repository.dart';
import 'package:path/path.dart' as p;

enum Menu { download, move, rename, delete, emptyTrash }

class FileExplorerView extends StatelessWidget {
  const FileExplorerView({super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder(
      init: FileExplorerController(),
      builder: (fileExplorerController) {
        final entities =
            fileExplorerController.entities
                .where(
                  (e) =>
                      p.dirname(e.path) == Repository.to.fileExplorerViewPath,
                )
                .toList();

        return DataTable(
          sortColumnIndex: fileExplorerController.sortColumnIndex,
          sortAscending: fileExplorerController.sortAscending,
          columns: [
            DataColumn(
              label: Text("Name"),
              columnWidth: IntrinsicColumnWidth(flex: 1),
              onSort: fileExplorerController.onSort,
            ),
            DataColumn(
              label: Text("Modified"),
              onSort: fileExplorerController.onSort,
            ),
            DataColumn(
              label: Text("Size"),
              onSort: fileExplorerController.onSort,
            ),
            DataColumn(label: Container()),
          ],
          rows: List.generate(entities.length, (index) {
            final entity = entities[index];

            Widget preview = Container();
            if (entity.kind == "folder") {
              preview = Icon(
                Icons.folder,
                size: 16,
                color: Theme.of(context).colorScheme.primary,
              );
            }
            if (entity.kind == "x") {
              String? mime = entity.tags.elementAtOrNull(4);
              if (mime != null && mime.split("/").first == "image") {
                preview = FutureBuilder(
                  future: entity.download(),
                  builder: (context, snapshot) {
                    if (snapshot.data == null) {
                      return Container();
                    }
                    return ClipRRect(
                      borderRadius: BorderRadius.circular(4),
                      child: Image.memory(snapshot.data!, fit: BoxFit.cover),
                    );
                  },
                );
              } else {
                preview = Icon(
                  Icons.insert_drive_file,
                  size: 16,
                  color: Theme.of(context).colorScheme.primary,
                );
              }
            }

            return DataRow(
              cells: [
                DataCell(
                  Row(
                    children: [
                      SizedBox(width: 16, height: 16, child: preview),
                      SizedBox(width: 8),
                      Text(entity.name),
                    ],
                  ),
                  onTap: () {
                    if (entity.kind == "folder") {
                      Repository.to.fileExplorerViewPath = entity.path;
                    }
                  },
                ),
                DataCell(Text(DateFormat().format(entity.createdAt))),
                DataCell(Text(formatBytes(entity.size))),
                DataCell(
                  PopupMenuButton(
                    itemBuilder:
                        (context) => <PopupMenuEntry<Menu>>[
                          if (entity.path != trashPath)
                            PopupMenuItem(
                              value: Menu.download,
                              child: ListTile(
                                leading: Icon(Icons.download),
                                title: Text("Download"),
                              ),
                            ),
                          if (entity.path != trashPath)
                            PopupMenuItem(
                              value: Menu.move,
                              child: ListTile(
                                leading: Icon(Icons.drive_file_move_outlined),
                                title: Text("Move to folder"),
                              ),
                            ),
                          if (entity.path != trashPath)
                            PopupMenuItem(
                              value: Menu.rename,
                              child: ListTile(
                                leading: Icon(
                                  Icons.drive_file_rename_outline_rounded,
                                ),
                                title: Text("Rename"),
                              ),
                            ),
                          if (entity.path != trashPath)
                            PopupMenuItem(
                              value: Menu.delete,
                              child: ListTile(
                                leading: Icon(Icons.delete_outlined),
                                title: Text("Move to trash"),
                              ),
                            ),
                          if (entity.path == trashPath)
                            PopupMenuItem(
                              value: Menu.emptyTrash,
                              child: ListTile(
                                leading: Icon(Icons.delete_outlined),
                                title: Text("Empty trash"),
                              ),
                            ),
                        ],
                    onSelected:
                        (value) => fileExplorerController.onEntityMenuSelected(
                          value,
                          entity,
                        ),
                  ),
                ),
              ],
              onSelectChanged: (value) {},
            );
          }),
        );
      },
    );
  }
}
