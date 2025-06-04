import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/drop_zone/drop_zone_controller.dart';

class DropZoneView extends StatelessWidget {
  final Widget child;

  const DropZoneView({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    return DropTarget(
      onDragDone: DropZoneController.onDragDone,
      onDragEntered: DropZoneController.onDragEntered,
      onDragExited: DropZoneController.onDragExited,
      child: GetBuilder(
        init: Repository.to,
        builder: (repository) {
          if (repository.isDraggingFile) {
            return Scaffold(
              body: Center(
                child: Text(
                  "Drop here",
                  style: Theme.of(context).textTheme.displayLarge,
                ),
              ),
            );
          }
          return child;
        },
      ),
    );
  }
}
