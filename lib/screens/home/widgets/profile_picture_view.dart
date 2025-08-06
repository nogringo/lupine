import 'package:flutter/material.dart';
import 'package:lupine/repository.dart';

class ProfilePictureView extends StatelessWidget {
  const ProfilePictureView({super.key});

  @override
  Widget build(BuildContext context) {
    return FutureBuilder(
      future: Repository.to.driveService.ndk.metadata.loadMetadata(
        Repository.to.driveService.ndk.accounts.getPublicKey()!,
      ),
      builder: (context, snapshot) {
        final pubkey = Repository.to.driveService.ndk.accounts.getPublicKey()!;
        String? pictureUrl;
        if (snapshot.data != null) {
          pictureUrl = snapshot.data!.picture;
        }
        pictureUrl ??= "https://robohash.org/$pubkey";
        return CircleAvatar(backgroundImage: NetworkImage(pictureUrl));
      },
    );
  }
}
