import 'package:file_saver/file_saver.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:nip19/nip19.dart';
import 'package:nip49/nip49.dart';
import 'package:toastification/toastification.dart';

class ShareController extends GetxController {
  static ShareController get to => Get.find();

  final passwordController = TextEditingController();
  RxBool isDownloading = false.obs;
  String? privateKey;
  SharedFileAccess? linkInfo;
  FileMetadata? driveItem;

  bool get isLocked => privateKey == null;
  bool get isLoading => driveItem == null;

  ShareController() {
    init();
  }

  void init() {
    linkInfo = parseShareLink(Get.currentRoute);

    if (linkInfo!.isPasswordProtected) {
      update();
      return;
    }

    privateKey = Nip19.nsecToHex(linkInfo!.encodedPrivateKey);
    loadFile();

    update();
  }

  Future<void> unlock() async {
    try {
      privateKey = await Nip49.decrypt(
        linkInfo!.encodedPrivateKey,
        passwordController.text,
      );
      passwordController.clear();
      update();
      loadFile();
    } catch (e) {
      toastification.show(
        title: Text('Incorrect password'),
        type: ToastificationType.error,
        autoCloseDuration: Duration(seconds: 3),
        alignment: Alignment.bottomRight,
      );
    }
  }

  Future<void> loadFile() async {
    if (privateKey == null || linkInfo == null) return;

    driveItem = await accessSharedFile(
      nevent: linkInfo!.nevent,
      privateKey: privateKey!,
    );

    update();
  }

  Future<void> downloadFile() async {
    if (linkInfo == null || driveItem == null) return;

    isDownloading.value = true;

    try {
      final hostingBlossomServers = await Repository
          .to
          .ndk
          .blossomUserServerList
          .getUserServerList(pubkeys: [linkInfo!.author]);

      if (hostingBlossomServers == null) {
        toastification.show(
          title: Text('Failed to fetch server list'),
          type: ToastificationType.error,
          autoCloseDuration: Duration(seconds: 3),
          alignment: Alignment.bottomRight,
        );
        return;
      }

      DecryptionInfo? decryptionInfo;
      if (driveItem!.decryptionKey != null &&
          driveItem!.decryptionNonce != null) {
        decryptionInfo = DecryptionInfo(
          key: driveItem!.decryptionKey!,
          nonce: driveItem!.decryptionNonce!,
        );
      }

      final bytes = await downloadFileFromBlossom(
        hash: driveItem!.hash,
        blossomServers: hostingBlossomServers,
        decryptionInfo: decryptionInfo,
      );

      // Save the file using FileSaver
      await FileSaver.instance.saveFile(
        name: driveItem!.name,
        bytes: bytes,
        mimeType: MimeType.other,
      );

      toastification.show(
        title: Text('File downloaded successfully'),
        type: ToastificationType.success,
        autoCloseDuration: Duration(seconds: 3),
        alignment: Alignment.bottomRight,
      );
    } catch (e) {
      toastification.show(
        title: Text('Download failed'),
        description: Text(e.toString()),
        type: ToastificationType.error,
        autoCloseDuration: Duration(seconds: 3),
        alignment: Alignment.bottomRight,
      );
    } finally {
      isDownloading.value = false;
    }
  }
}
