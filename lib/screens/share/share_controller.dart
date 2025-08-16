import 'dart:async';

import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/repository.dart';
import 'package:lupine_sdk/lupine_sdk.dart';
import 'package:ndk/ndk.dart';
import 'package:toastification/toastification.dart';

class ShareController extends GetxController {
  final DriveItem entity;

  ShareController({required this.entity});

  // User search
  final searchController = TextEditingController();
  final RxList<Metadata> searchResults = <Metadata>[].obs;
  final RxList<Metadata> followingList = <Metadata>[].obs;
  final RxBool isSearching = false.obs;
  final RxBool isLoadingFollowing = false.obs;
  Timer? _debounceTimer;

  // Selected user
  final Rxn<String> selectedUserPubkey = Rxn<String>();
  final Rxn<String> selectedUserName = Rxn<String>();
  final pubkeyController = TextEditingController();
  final RxBool canShare = false.obs;

  // Share link
  final Rxn<String> shareLink = Rxn<String>();
  final RxBool isGeneratingLink = false.obs;

  @override
  void onInit() {
    super.onInit();
    searchController.addListener(_onSearchChanged);

    // Update selected user when pubkey is entered manually
    pubkeyController.addListener(() {
      final value = pubkeyController.text;
      canShare.value = value.isNotEmpty;
      if (value.isNotEmpty && selectedUserPubkey.value != value) {
        selectedUserPubkey.value = null;
        selectedUserName.value = null;
      }
    });
    
    // Load following list when dialog opens
    loadFollowingList();
  }

  @override
  void onClose() {
    searchController.dispose();
    pubkeyController.dispose();
    _debounceTimer?.cancel();
    super.onClose();
  }

  void _onSearchChanged() {
    _debounceTimer?.cancel();

    if (searchController.text.isEmpty) {
      searchResults.clear();
      selectedUserPubkey.value = null;
      selectedUserName.value = null;
      return;
    }

    _debounceTimer = Timer(const Duration(milliseconds: 500), () {
      searchUsers(searchController.text);
    });
  }

  Future<void> loadFollowingList() async {
    isLoadingFollowing.value = true;
    followingList.clear();
    
    try {
      // Get the logged-in user's account
      final account = Repository.to.ndk.accounts.getLoggedAccount();
      if (account == null) return;
      
      // Get the contact list for the logged-in user
      final contactList = await Repository.to.ndk.follows.getContactList(account.pubkey);
      if (contactList == null || contactList.contacts.isEmpty) {
        isLoadingFollowing.value = false;
        return;
      }
      
      // Load metadata for all contacts
      final metadatas = await Repository.to.ndk.metadata.loadMetadatas(
        contactList.contacts,
        null, // No specific relays
      );
      
      // Filter out null values and sort by name
      final validMetadatas = metadatas.cast<Metadata>().toList();
      validMetadatas.sort((a, b) {
        final aName = (a.name ?? a.displayName ?? '').toLowerCase();
        final bName = (b.name ?? b.displayName ?? '').toLowerCase();
        return aName.compareTo(bName);
      });
      
      followingList.value = validMetadatas;
    } catch (e) {
      debugPrint('Error loading following list: $e');
    } finally {
      isLoadingFollowing.value = false;
    }
  }

  Future<void> searchUsers(String query) async {
    if (query.isEmpty) return;

    isSearching.value = true;
    searchResults.clear();

    try {
      // Request metadata events (kind 0) with search parameter
      final request = Repository.to.ndk.requests.query(
        filters: [
          Filter(
            kinds: [0], // Metadata events
            search: query, // Use the search parameter to filter on relays
            limit: 10,
          ),
        ],
      );

      final List<Metadata> results = [];

      await for (final event in request.stream) {
        try {
          // Parse the metadata from the event
          final metadata = Metadata.fromEvent(event);
          results.add(metadata);

          // Update results as they come in
          searchResults.value = List.from(results);

          // Stop if we have enough results
          if (results.length >= 10) break;
        } catch (e) {
          // Skip invalid metadata events
          continue;
        }
      }
    } catch (e) {
      debugPrint('Error searching users: $e');
    } finally {
      isSearching.value = false;
    }
  }

  void selectUser(Metadata user) {
    selectedUserPubkey.value = user.pubKey;
    selectedUserName.value = user.name ?? user.displayName ?? 'Unknown';
    pubkeyController.text = user.pubKey;
    canShare.value = true;
    searchController.clear();
    searchResults.clear();
  }

  void clearSelectedUser() {
    selectedUserPubkey.value = null;
    selectedUserName.value = null;
    pubkeyController.clear();
    canShare.value = false;
  }

  Future<void> generateShareLink() async {
    isGeneratingLink.value = true;

    try {
      final link = await Repository.to.generateShareLink(entity);
      shareLink.value = link;
    } catch (e) {
      toastification.show(
        context: Get.context,
        type: ToastificationType.error,
        style: ToastificationStyle.fillColored,
        alignment: Alignment.bottomRight,
        title: const Text("Error"),
        description: Text("Failed to generate share link: $e"),
      );
    } finally {
      isGeneratingLink.value = false;
    }
  }

  Future<void> shareWithUser() async {
    final pubkey = pubkeyController.text.trim();
    if (pubkey.isEmpty) return;

    try {
      await Repository.to.shareWithPubkey(entity, pubkey);
      Get.back();
    } catch (e) {
      toastification.show(
        context: Get.context,
        type: ToastificationType.error,
        style: ToastificationStyle.fillColored,
        alignment: Alignment.bottomRight,
        title: const Text("Error"),
        description: Text("Failed to share: $e"),
      );
    }
  }
}
