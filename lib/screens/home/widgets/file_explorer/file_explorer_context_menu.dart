import 'package:flutter/material.dart';
import 'package:lupine/config.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_view.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class FileExplorerContextMenu {
  static void showAtPosition({
    required BuildContext context,
    required RelativeRect position,
    required DriveItem entity,
    required Function(Menu, DriveItem) onMenuSelected,
  }) {
    final isInTrashcan = entity.parentPath == trashPath;
    showMenu(
      context: context,
      position: position,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(color: Theme.of(context).dividerColor, width: 1),
      ),
      items: buildMenuItems(context, entity, isInTrashcan),
    ).then((value) {
      if (value != null) {
        onMenuSelected(value, entity);
      }
    });
  }

  static Future<Menu?> showAsPopup({
    required BuildContext context,
    required DriveItem entity,
  }) {
    final isInTrashcan = entity.parentPath == trashPath;
    return showMenu<Menu>(
      context: context,
      position: RelativeRect.fromLTRB(0, 0, 0, 0),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(color: Theme.of(context).dividerColor, width: 1),
      ),
      items: buildMenuItems(context, entity, isInTrashcan),
    );
  }

  static List<PopupMenuEntry<Menu>> buildMenuItems(
    BuildContext context,
    DriveItem entity,
    bool isInTrashcan,
  ) {
    return [
      if (!isInTrashcan)
        PopupMenuItem(
          value: Menu.info,
          child: ListTile(
            leading: Icon(Icons.info_outline),
            title: Text("Info"),
          ),
        ),
      if (!isInTrashcan)
        PopupMenuItem(
          value: Menu.download,
          child: ListTile(
            leading: Icon(Icons.download),
            title: Text("Download"),
          ),
        ),
      if (!isInTrashcan)
        PopupMenuItem(
          value: Menu.rename,
          child: ListTile(
            leading: Icon(Icons.drive_file_rename_outline_rounded),
            title: Text("Rename"),
          ),
        ),
      if (!isInTrashcan && entity.isFile)
        PopupMenuItem(
          value: Menu.share,
          child: ListTile(leading: Icon(Icons.share), title: Text("Share")),
        ),
      if (!isInTrashcan)
        PopupMenuItem(
          value: Menu.move,
          child: ListTile(
            leading: Icon(Icons.drive_file_move_outlined),
            title: Text("Move"),
          ),
        ),
      if (!isInTrashcan && entity.isFile)
        PopupMenuItem(
          value: Menu.changelog,
          child: ListTile(
            leading: Icon(Icons.restore),
            title: Text("Changelog"),
          ),
        ),
      if (!isInTrashcan)
        PopupMenuItem(
          value: Menu.delete,
          child: ListTile(
            leading: Icon(Icons.delete_outlined),
            title: Text("Move to trash"),
          ),
        ),
      if (isInTrashcan)
        PopupMenuItem(
          value: Menu.restore,
          child: ListTile(
            leading: Icon(Icons.restore_from_trash),
            title: Text("Restore"),
          ),
        ),
      if (isInTrashcan)
        PopupMenuItem(
          value: Menu.deleteForever,
          child: ListTile(
            leading: Icon(Icons.delete_forever),
            title: Text("Delete forever"),
          ),
        ),
    ];
  }
}
