import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/share/share_controller.dart';
import 'package:ndk/ndk.dart';

class UserSearchSection extends StatelessWidget {
  final ShareController controller;

  const UserSearchSection({super.key, required this.controller});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          'Share with Nostr user',
          style: Theme.of(context).textTheme.titleMedium,
        ),
        const SizedBox(height: 8),
        _buildSearchField(context),
        Obx(() {
          if (controller.isSearching.value) {
            return const Padding(
              padding: EdgeInsets.symmetric(vertical: 8.0),
              child: Center(child: CircularProgressIndicator()),
            );
          }

          if (controller.searchResults.isNotEmpty) {
            return _buildSearchResults(context);
          }
          
          // Show following list when no search is active
          if (controller.searchController.text.isEmpty && controller.followingList.isNotEmpty) {
            return _buildFollowingList(context);
          }
          
          if (controller.isLoadingFollowing.value) {
            return const Padding(
              padding: EdgeInsets.symmetric(vertical: 8.0),
              child: Center(child: CircularProgressIndicator()),
            );
          }

          return const SizedBox.shrink();
        }),
        Obx(() {
          if (controller.selectedUserName.value != null) {
            return _buildSelectedUser(context);
          }
          return const SizedBox.shrink();
        }),
        const SizedBox(height: 8),
        Text(
          'Or enter public key directly:',
          style: Theme.of(context).textTheme.bodySmall,
        ),
        const SizedBox(height: 4),
        _buildPubkeyField(context),
      ],
    );
  }

  Widget _buildSearchField(BuildContext context) {
    return TextField(
      controller: controller.searchController,
      decoration: InputDecoration(
        hintText: 'Search by name or username',
        prefixIcon: const Icon(Icons.search),
        border: OutlineInputBorder(borderRadius: BorderRadius.circular(8)),
        contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      ),
    );
  }

  Widget _buildSearchResults(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(top: 8),
      decoration: BoxDecoration(
        border: Border.all(color: Theme.of(context).dividerColor),
        borderRadius: BorderRadius.circular(8),
      ),
      constraints: const BoxConstraints(maxHeight: 200),
      child: Obx(
        () => ListView.builder(
          shrinkWrap: true,
          itemCount: controller.searchResults.length,
          itemBuilder: (context, index) {
            final user = controller.searchResults[index];
            return _buildUserTile(context, user);
          },
        ),
      ),
    );
  }
  
  Widget _buildFollowingList(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 8.0),
          child: Text(
            'People you follow',
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
        ),
        Container(
          decoration: BoxDecoration(
            border: Border.all(color: Theme.of(context).dividerColor),
            borderRadius: BorderRadius.circular(8),
          ),
          constraints: const BoxConstraints(maxHeight: 200),
          child: Obx(
            () => ListView.builder(
              shrinkWrap: true,
              itemCount: controller.followingList.length,
              itemBuilder: (context, index) {
                final user = controller.followingList[index];
                return _buildUserTile(context, user);
              },
            ),
          ),
        ),
      ],
    );
  }

  Widget _buildUserTile(BuildContext context, Metadata user) {
    final name = user.name ?? user.displayName ?? 'Unknown';
    final nip05 = user.nip05;

    return ListTile(
      leading: CircleAvatar(
        backgroundImage:
            user.picture != null ? NetworkImage(user.picture!) : null,
        child:
            user.picture == null
                ? Text(name.isNotEmpty ? name[0].toUpperCase() : '?')
                : null,
      ),
      title: Text(name),
      subtitle: nip05 != null ? Text(nip05) : null,
      onTap: () => controller.selectUser(user),
    );
  }

  Widget _buildSelectedUser(BuildContext context) {
    return Container(
      margin: const EdgeInsets.only(top: 8),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Theme.of(context).colorScheme.primaryContainer,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        children: [
          Icon(
            Icons.person,
            color: Theme.of(context).colorScheme.onPrimaryContainer,
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Selected: ${controller.selectedUserName.value}',
                  style: TextStyle(
                    fontWeight: FontWeight.bold,
                    color: Theme.of(context).colorScheme.onPrimaryContainer,
                  ),
                ),
                Text(
                  '${controller.selectedUserPubkey.value!.substring(0, 16)}...',
                  style: TextStyle(
                    fontSize: 12,
                    color: Theme.of(context).colorScheme.onPrimaryContainer,
                  ),
                ),
              ],
            ),
          ),
          IconButton(
            icon: Icon(
              Icons.clear,
              color: Theme.of(context).colorScheme.onPrimaryContainer,
            ),
            onPressed: controller.clearSelectedUser,
          ),
        ],
      ),
    );
  }

  Widget _buildPubkeyField(BuildContext context) {
    return TextField(
      controller: controller.pubkeyController,
      decoration: InputDecoration(
        hintText: 'npub or hex public key',
        border: OutlineInputBorder(borderRadius: BorderRadius.circular(8)),
        contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      ),
    );
  }
}
