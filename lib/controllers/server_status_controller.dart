import 'package:get/get.dart';
import 'package:lupine/repository.dart';

class ServerStatusController extends GetxController {
  static ServerStatusController get to => Get.find();

  bool _needsConfiguration = false;
  bool get needsConfiguration => _needsConfiguration;

  bool _hasNoBlossomServers = false;
  bool get hasNoBlossomServers => _hasNoBlossomServers;

  @override
  void onInit() {
    super.onInit();
    checkServerStatus();
  }

  Future<void> checkServerStatus() async {
    try {
      final pubkey = Repository.to.ndk.accounts.getPublicKey();
      if (pubkey == null) {
        _needsConfiguration = false;
        _hasNoBlossomServers = false;
        update();
        return;
      }

      // Check relays count
      final userRelayLists = await Repository.to.ndk.userRelayLists
          .getSingleUserRelayList(pubkey);
      final relayCount = userRelayLists?.relays.keys.length ?? 0;

      // Check blossom servers count
      final blossomUserServerList = await Repository
          .to
          .driveService
          .ndk
          .blossomUserServerList
          .getUserServerList(pubkeys: [pubkey]);
      final blossomCount = blossomUserServerList?.length ?? 0;

      // Track if user has NO blossom servers (critical issue)
      _hasNoBlossomServers = blossomCount == 0;

      // Need configuration if less than 2 relays OR less than 2 blossom servers
      _needsConfiguration = relayCount < 2 || blossomCount < 2;
      update();
    } catch (e) {
      _needsConfiguration = false;
      _hasNoBlossomServers = false;
      update();
    }
  }
}
