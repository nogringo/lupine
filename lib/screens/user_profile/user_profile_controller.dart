import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/controllers/server_status_controller.dart';
import 'package:lupine/repository.dart';
import 'package:ndk/entities.dart';

class UserProfileController extends GetxController {
  static UserProfileController get to => Get.find();

  final TextEditingController relayFieldController = TextEditingController();
  final TextEditingController blossomServerFieldController =
      TextEditingController();

  List<String> relaysUrl = [];
  List<String> blossomServersUrl = [];
  List<String> originalRelaysUrl = [];
  List<String> originalBlossomServersUrl = [];
  RxBool isLoading = false.obs;
  RxBool relaysModified = false.obs;
  RxBool blossomServersModified = false.obs;

  bool _showDiscoverRelays = false;
  bool get showDiscoverRelays => _showDiscoverRelays;
  set showDiscoverRelays(bool value) {
    _showDiscoverRelays = value;
    update();
  }

  bool _showDiscoverBlossomServers = false;
  bool get showDiscoverBlossomServers => _showDiscoverBlossomServers;
  set showDiscoverBlossomServers(bool value) {
    _showDiscoverBlossomServers = value;
    update();
  }

  @override
  void onInit() {
    super.onInit();
    loadServers();
  }

  @override
  void onClose() {
    relayFieldController.dispose();
    blossomServerFieldController.dispose();
    super.onClose();
  }

  Future<void> loadServers() async {
    isLoading.value = true;
    try {
      final pubkey = Repository.to.ndk.accounts.getPublicKey()!;

      final userRelayLists = await Repository.to.ndk.userRelayLists
          .getSingleUserRelayList(pubkey);

      if (userRelayLists != null) {
        relaysUrl = userRelayLists.relays.keys.toList();
        originalRelaysUrl = List.from(relaysUrl);
      }

      final blossomUserServerList = await Repository
          .to
          .driveService
          .ndk
          .blossomUserServerList
          .getUserServerList(pubkeys: [pubkey]);

      if (blossomUserServerList != null) {
        blossomServersUrl = blossomUserServerList;
        originalBlossomServersUrl = List.from(blossomServersUrl);
      }

      update();
    } finally {
      isLoading.value = false;
    }
  }

  bool addRelay(String url) {
    final alreadyExist = relaysUrl.contains(url);
    if (alreadyExist) return false;

    final uri = Uri.tryParse(url);
    if (uri == null) return false;
    if (uri.scheme != "wss" && uri.scheme != "ws") return false;

    relaysUrl.add(url);
    _checkRelaysModified();
    update();

    return true;
  }

  void removeRelay(String url) {
    relaysUrl.remove(url);
    _checkRelaysModified();
    update();
  }

  bool addBlossomServer(String url) {
    final alreadyExist = blossomServersUrl.contains(url);
    if (alreadyExist) return false;

    final uri = Uri.tryParse(url);
    if (uri == null) return false;
    if (uri.scheme != "https" && uri.scheme != "http") return false;

    blossomServersUrl.add(url);
    _checkBlossomServersModified();
    update();

    return true;
  }

  void removeBlossomServer(String url) {
    blossomServersUrl.remove(url);
    _checkBlossomServersModified();
    update();
  }

  void addRelayFromField() {
    final url = relayFieldController.text.trim();
    if (url.isNotEmpty) {
      final success = addRelay(url);
      if (success) relayFieldController.clear();
    }
  }

  void addBlossomServerFromField() {
    final url = blossomServerFieldController.text.trim();
    if (url.isNotEmpty) {
      final success = addBlossomServer(url);
      if (success) blossomServerFieldController.clear();
    }
  }

  bool get hasInsufficientRelays => relaysUrl.length < 2;
  bool get hasNoBlossomServers => blossomServersUrl.isEmpty;
  bool get needsConfiguration => hasInsufficientRelays || hasNoBlossomServers;

  void _checkRelaysModified() {
    relaysModified.value = !_listEquals(relaysUrl, originalRelaysUrl);
  }

  void _checkBlossomServersModified() {
    blossomServersModified.value =
        !_listEquals(blossomServersUrl, originalBlossomServersUrl);
  }

  bool _listEquals(List<String> a, List<String> b) {
    if (a.length != b.length) return false;
    final sortedA = List<String>.from(a)..sort();
    final sortedB = List<String>.from(b)..sort();
    for (int i = 0; i < sortedA.length; i++) {
      if (sortedA[i] != sortedB[i]) return false;
    }
    return true;
  }

  Future<void> saveRelays() async {
    if (!relaysModified.value) return;

    isLoading.value = true;
    try {
      // Convert relay URLs to a map with ReadWriteMarker
      final relaysMap = <String, ReadWriteMarker>{};
      for (final relay in relaysUrl) {
        relaysMap[relay] = ReadWriteMarker.readWrite;
      }

      await Repository.to.ndk.userRelayLists.setInitialUserRelayList(
        UserRelayList(
          pubKey: Repository.to.ndk.accounts.getPublicKey()!,
          relays: relaysMap,
          createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
          refreshedTimestamp: DateTime.now().millisecondsSinceEpoch ~/ 1000,
        ),
      );

      originalRelaysUrl = List.from(relaysUrl);
      relaysModified.value = false;

      // Update server status
      if (Get.isRegistered<ServerStatusController>()) {
        ServerStatusController.to.checkServerStatus();
      }
    } finally {
      isLoading.value = false;
    }
  }

  Future<void> saveBlossomServers() async {
    if (!blossomServersModified.value) return;

    isLoading.value = true;
    try {
      // Publish the updated server list
      Repository.to.ndk.blossomUserServerList.publishUserServerList(
        serverUrlsOrdered: blossomServersUrl,
      );

      originalBlossomServersUrl = List.from(blossomServersUrl);
      blossomServersModified.value = false;

      // Update server status
      if (Get.isRegistered<ServerStatusController>()) {
        ServerStatusController.to.checkServerStatus();
      }
    } finally {
      isLoading.value = false;
    }
  }
}
