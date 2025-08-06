import 'package:flutter/material.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class EntityThumbnailView extends StatelessWidget {
  final DriveItem entity;
  const EntityThumbnailView(this.entity, {super.key});

  @override
  Widget build(BuildContext context) {
    final folderPreview = Icon(
      Icons.folder,
      size: 16,
      color: Theme.of(context).colorScheme.primary,
    );

    final filePreview = Icon(
      Icons.insert_drive_file,
      size: 16,
      color: Theme.of(context).colorScheme.primary,
    );

    if (entity is FolderMetadata) return folderPreview;

    // String? mime = (entity as FileMetadata).fileType;
    // if (kIsWeb && mime == "image/x-icon") return filePreview;

    // if (mime != null && mime.split("/").first == "image") {
    //   return FutureBuilder(
    //     future: Repository.to.driveService.downloadFile(hash: (entity as FileMetadata).hash),
    //     builder: (context, snapshot) {
    //       if (snapshot.data == null) {
    //         return Container();
    //       }
    //       return ClipRRect(
    //         borderRadius: BorderRadius.circular(4),
    //         child: Image.memory(snapshot.data!, fit: BoxFit.cover),
    //       );
    //     },
    //   );
    // }

    return filePreview;
  }
}
