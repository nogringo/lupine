import 'package:flutter/material.dart' hide Action;
import 'package:get/get.dart';
import 'package:intl/intl.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/changelog_dialog/changelog_dialog_controller.dart';
import 'package:lupine/screens/home/widgets/entity_thumbnail_view.dart';
import 'package:lupine/utils/format_bytes.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class ChangelogDialogView extends StatelessWidget {
  final FileMetadata entity;
  const ChangelogDialogView(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(ChangelogDialogController());
    return AlertDialog(
      scrollable: true,
      contentPadding: EdgeInsets.all(0),
      title: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [Text(entity.name), CloseButton()],
      ),
      content: FutureBuilder(
        future: Repository.to.driveService.getFileVersions(entity.path),
        builder: (context, snapshot) {
          if (!snapshot.hasData) return Container();

          final versions = snapshot.data!;

          return GetBuilder<ChangelogDialogController>(
            builder: (c) {
              return Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  ...versions.map(
                    (e) => ListTile(
                      leading: SizedBox(
                        height: 32,
                        width: 32,
                        child: EntityThumbnailView(entity),
                      ),
                      title: Text(
                        DateFormat().format(e.createdAt),
                        maxLines: 1,
                      ),
                      subtitle: Text(formatBytes(e.size)),
                      trailing: IconButton(
                        onPressed: () {},
                        icon: Icon(Icons.more_vert),
                      ),
                    ),
                  ),
                ],
              );
            },
          );
        },
      ),
    );
  }
}
