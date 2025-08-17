import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/user_profile/user_profile_controller.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:nostr_widgets/nostr_widgets.dart';
import 'package:window_manager/window_manager.dart';

class UserProfilePage extends StatelessWidget {
  const UserProfilePage({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = Get.put(UserProfileController());

    return Scaffold(
      appBar: isDesktop
          ? PreferredSize(
              preferredSize: Size.fromHeight(kToolbarHeight),
              child: DragToMoveArea(
                child: AppBar(
                  title: Text('Profile'),
                  actions: [
                    Align(
                      alignment: Alignment.topCenter,
                      child: WindowButtons(),
                    ),
                  ],
                ),
              ),
            )
          : AppBar(
              title: Text('Profile'),
            ),
      body: SingleChildScrollView(
        padding: EdgeInsets.all(16),
        child: Center(
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: 600),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // User Profile Card
                NUserProfile(
                  ndk: Repository.to.ndk,
                  onLogout: () {
                    Repository.to.logout();
                  },
                ),
                SizedBox(height: 24),

                // Relays Section
                Container(
                  height: 400,
                  decoration: BoxDecoration(
                    border: Border.all(
                      color: Theme.of(
                        context,
                      ).colorScheme.outline.withValues(alpha: 0.6),
                      width: 2,
                    ),
                    borderRadius: BorderRadius.circular(40),
                  ),
                  padding: EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Text(
                            'Nostr Relays',
                            style: Theme.of(context).textTheme.titleLarge,
                          ),
                          Obx(
                            () =>
                                controller.relaysModified.value
                                    ? ElevatedButton(
                                      onPressed: controller.saveRelays,
                                      child: Text('Save'),
                                    )
                                    : Container(),
                          ),
                        ],
                      ),
                      SizedBox(height: 8),
                      // List of relays in the middle
                      Expanded(
                        child: Obx(() {
                          if (controller.relaysLoading.value) {
                            return Center(
                              child: CircularProgressIndicator(),
                            );
                          }
                          
                          return GetBuilder<UserProfileController>(
                            builder: (c) {
                              if (c.relaysUrl.isEmpty) {
                              return Container(
                                padding: EdgeInsets.all(24),
                                alignment: Alignment.center,
                                child: Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    Icon(
                                      Icons.warning_amber_rounded,
                                      size: 48,
                                      color:
                                          Theme.of(context).colorScheme.error,
                                    ),
                                    SizedBox(height: 8),
                                    Text(
                                      'No relays configured',
                                      style:
                                          Theme.of(
                                            context,
                                          ).textTheme.bodyMedium,
                                    ),
                                  ],
                                ),
                              );
                            }

                            return ListView(
                              shrinkWrap: true,
                              children:
                                  c.relaysUrl
                                      .map(
                                        (relay) => ListTile(
                                          leading: Icon(Icons.dns),
                                          title: Text(relay),
                                          trailing: IconButton(
                                            icon: Icon(Icons.close),
                                            onPressed: () {
                                              showDialog(
                                                context: context,
                                                builder:
                                                    (context) => AlertDialog(
                                                      title: Text(
                                                        'Remove Relay',
                                                      ),
                                                      content: Text(
                                                        'Are you sure you want to remove this relay?\n\n$relay',
                                                      ),
                                                      actions: [
                                                        TextButton(
                                                          onPressed:
                                                              () =>
                                                                  Navigator.pop(
                                                                    context,
                                                                  ),
                                                          child: Text('Cancel'),
                                                        ),
                                                        TextButton(
                                                          onPressed: () {
                                                            Navigator.pop(
                                                              context,
                                                            );
                                                            c.removeRelay(
                                                              relay,
                                                            );
                                                          },
                                                          child: Text(
                                                            'Remove',
                                                            style: TextStyle(
                                                              color:
                                                                  Theme.of(
                                                                        context,
                                                                      )
                                                                      .colorScheme
                                                                      .error,
                                                            ),
                                                          ),
                                                        ),
                                                      ],
                                                    ),
                                              );
                                            },
                                          ),
                                        ),
                                      )
                                      .toList(),
                            );
                          },
                        );
                        }),
                      ),
                      SizedBox(height: 12),
                      // Text field at the bottom with stadium border
                      TextField(
                        controller: controller.relayFieldController,
                        decoration: InputDecoration(
                          labelText: 'Add Relay',
                          hintText: 'wss://relay.example.com',
                          enabledBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(30),
                            borderSide: BorderSide(
                              color: Theme.of(context).colorScheme.outline,
                            ),
                          ),
                          focusedBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(30),
                            borderSide: BorderSide(
                              color: Theme.of(context).colorScheme.primary,
                              width: 2,
                            ),
                          ),
                          contentPadding: EdgeInsets.symmetric(
                            horizontal: 20,
                            vertical: 16,
                          ),
                          suffixIcon: Padding(
                            padding: EdgeInsets.only(right: 8),
                            child: IconButton(
                              icon: Icon(Icons.add),
                              onPressed: controller.addRelayFromField,
                            ),
                          ),
                        ),
                        onSubmitted: (_) => controller.addRelayFromField(),
                      ),
                    ],
                  ),
                ),
                SizedBox(height: 16),

                // Blossom Servers Section
                Container(
                  height: 400,
                  decoration: BoxDecoration(
                    border: Border.all(
                      color: Theme.of(
                        context,
                      ).colorScheme.outline.withValues(alpha: 0.6),
                      width: 2,
                    ),
                    borderRadius: BorderRadius.circular(40),
                  ),
                  padding: EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Text(
                            'Blossom Servers',
                            style: Theme.of(context).textTheme.titleLarge,
                          ),
                          Obx(
                            () =>
                                controller.blossomServersModified.value
                                    ? ElevatedButton(
                                      onPressed: controller.saveBlossomServers,
                                      child: Text('Save'),
                                    )
                                    : Container(),
                          ),
                        ],
                      ),
                      SizedBox(height: 8),
                      // List of storage servers in the middle
                      Expanded(
                        child: Obx(() {
                          if (controller.blossomServersLoading.value) {
                            return Center(
                              child: CircularProgressIndicator(),
                            );
                          }
                          
                          return GetBuilder<UserProfileController>(
                            builder: (c) {
                              if (c.blossomServersUrl.isEmpty) {
                              return Container(
                                padding: EdgeInsets.all(24),
                                alignment: Alignment.center,
                                child: Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    Icon(
                                      Icons.error,
                                      size: 48,
                                      color:
                                          Theme.of(context).colorScheme.error,
                                    ),
                                    SizedBox(height: 8),
                                    Text(
                                      'No storage servers configured',
                                      style:
                                          Theme.of(
                                            context,
                                          ).textTheme.bodyMedium,
                                    ),
                                    Text(
                                      'Files cannot be stored without servers',
                                      style: Theme.of(
                                        context,
                                      ).textTheme.bodySmall?.copyWith(
                                        color:
                                            Theme.of(context).colorScheme.error,
                                      ),
                                    ),
                                  ],
                                ),
                              );
                            }

                            return ListView(
                              shrinkWrap: true,
                              children:
                                  c.blossomServersUrl
                                      .map(
                                        (server) => ListTile(
                                          leading: Icon(Icons.storage),
                                          title: Text(server),
                                          trailing: IconButton(
                                            icon: Icon(Icons.close),
                                            onPressed: () {
                                              showDialog(
                                                context: context,
                                                builder:
                                                    (context) => AlertDialog(
                                                      title: Text(
                                                        'Remove Storage Server',
                                                      ),
                                                      content: Text(
                                                        'Are you sure you want to remove this storage server?\n\n$server',
                                                      ),
                                                      actions: [
                                                        TextButton(
                                                          onPressed:
                                                              () =>
                                                                  Navigator.pop(
                                                                    context,
                                                                  ),
                                                          child: Text('Cancel'),
                                                        ),
                                                        TextButton(
                                                          onPressed: () {
                                                            Navigator.pop(
                                                              context,
                                                            );
                                                            c.removeBlossomServer(
                                                              server,
                                                            );
                                                          },
                                                          child: Text(
                                                            'Remove',
                                                            style: TextStyle(
                                                              color:
                                                                  Theme.of(
                                                                        context,
                                                                      )
                                                                      .colorScheme
                                                                      .error,
                                                            ),
                                                          ),
                                                        ),
                                                      ],
                                                    ),
                                              );
                                            },
                                          ),
                                        ),
                                      )
                                      .toList(),
                            );
                          },
                        );
                        }),
                      ),
                      SizedBox(height: 12),
                      // Text field at the bottom with stadium border
                      TextField(
                        controller: controller.blossomServerFieldController,
                        decoration: InputDecoration(
                          labelText: 'Add Storage Server',
                          hintText: 'https://blossom.example.com',
                          enabledBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(30),
                            borderSide: BorderSide(
                              color: Theme.of(context).colorScheme.outline,
                            ),
                          ),
                          focusedBorder: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(30),
                            borderSide: BorderSide(
                              color: Theme.of(context).colorScheme.primary,
                              width: 2,
                            ),
                          ),
                          contentPadding: EdgeInsets.symmetric(
                            horizontal: 20,
                            vertical: 16,
                          ),
                          suffixIcon: Padding(
                            padding: EdgeInsets.only(right: 8),
                            child: IconButton(
                              icon: Icon(Icons.add),
                              onPressed: controller.addBlossomServerFromField,
                            ),
                          ),
                        ),
                        onSubmitted:
                            (_) => controller.addBlossomServerFromField(),
                      ),
                    ],
                  ),
                ),
                SizedBox(height: kToolbarHeight),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
