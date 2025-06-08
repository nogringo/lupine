import 'package:flutter/material.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class ProfilePictureView extends StatelessWidget {
  const ProfilePictureView({super.key});

  @override
  Widget build(BuildContext context) {
    return FutureBuilder(
      future: DriveService().ndk.metadata.loadMetadata(
        DriveService().ndk.accounts.getPublicKey()!,
      ),
      builder: (context, snapshot) {
        final pubkey = DriveService().ndk.accounts.getPublicKey()!;
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
