import 'package:flutter/material.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_controller.dart';
import 'package:lupine/screens/home/widgets/file_explorer/file_explorer_grid_item.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class FileExplorerGridView extends StatelessWidget {
  final List<DriveItem> entities;
  final FileExplorerController controller;

  const FileExplorerGridView({
    super.key,
    required this.entities,
    required this.controller,
  });

  @override
  Widget build(BuildContext context) {
    return GridView.builder(
      padding: const EdgeInsets.all(16),
      gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
        maxCrossAxisExtent: 200,
        childAspectRatio: 1,
        crossAxisSpacing: 16,
        mainAxisSpacing: 16,
      ),
      itemCount: entities.length,
      itemBuilder: (context, index) {
        final entity = entities[index];
        return FileExplorerGridItem(entity: entity, controller: controller);
      },
    );
  }
}
