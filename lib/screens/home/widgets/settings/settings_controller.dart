import 'package:flutter/widgets.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:ndk/entities.dart';

class SettingsController extends GetxController {
  static SettingsController get to => Get.find();

  List<String> relaysUrl = [];
  List<String> blossomServersUrl = [];

  final relayFieldController = TextEditingController();
  final blossomServerFieldController = TextEditingController();

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

  bool _showAdvancedOptions = false;
  bool get showAdvancedOptions => _showAdvancedOptions;
  set showAdvancedOptions(bool value) {
    _showAdvancedOptions = value;
    update();
  }

  SettingsController() {
    init();
  }

  void init() async {
    final pubkey = Repository.to.driveService.ndk.accounts.getPublicKey()!;

    final userRelayLists = await Repository.to.driveService.ndk.userRelayLists
        .getSingleUserRelayList(pubkey);

    if (userRelayLists != null) {
      relaysUrl = userRelayLists.relays.keys.toList();
    }

    final blossomUserServerList = await Repository
        .to
        .driveService
        .ndk
        .blossomUserServerList
        .getUserServerList(pubkeys: [pubkey]);

    if (blossomUserServerList != null) {
      blossomServersUrl = blossomUserServerList;
    }

    update();
  }

  bool addRelay(String url) {
    final alreadyExist = relaysUrl.contains(url);
    if (alreadyExist) return false;

    final uri = Uri.tryParse(url);
    if (uri == null) return false;
    if (uri.scheme != "wss" && uri.scheme != "ws") return false;

    relaysUrl.add(url);
    update();

    final broadcastRelays = Repository
        .to
        .driveService
        .ndk
        .relays
        .connectedRelays
        .map((e) => e.url);
    Repository.to.driveService.ndk.userRelayLists.broadcastAddNip65Relay(
      relayUrl: url,
      marker: ReadWriteMarker.readWrite,
      broadcastRelays: broadcastRelays,
    );

    return true;
  }

  void addRelayFromField() {
    final success = addRelay(relayFieldController.text);
    if (success) relayFieldController.clear();
  }

  void removeRelay(String url) {
    relaysUrl.remove(url);
    update();
  }

  bool addBlossomServer(String url) {
    final alreadyExist = blossomServersUrl.contains(url);
    if (alreadyExist) return false;

    final uri = Uri.tryParse(url);
    if (uri == null) return false;
    if (uri.scheme != "https" && uri.scheme != "http") return false;

    blossomServersUrl.add(url);
    update();

    Repository.to.driveService.ndk.blossomUserServerList.publishUserServerList(
      serverUrlsOrdered: blossomServersUrl,
    );

    return true;
  }

  void addBlossomServerFromField() {
    final success = addBlossomServer(blossomServerFieldController.text);
    if (success) blossomServerFieldController.clear();
  }

  void removeBlossomServer(String url) {
    blossomServersUrl.remove(url);
    update();
  }

  void logout() async {
    Repository.to.logout();
  }
}
