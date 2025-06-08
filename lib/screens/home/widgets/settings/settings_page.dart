import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/nostr_utils/pubkey_to_npub.dart';
import 'package:lupine/screens/home/widgets/settings/settings_controller.dart';
import 'package:lupine_sdk/lupine_sdk.dart';

class SettingsView extends StatelessWidget {
  const SettingsView({super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(SettingsController());
    return Align(
      alignment: Alignment.topCenter,
      child: SingleChildScrollView(
        child: Center(
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: 600),
            child: Column(
              children: [
                SizedBox(height: 16),
                UserView(),
                SizedBox(height: 16),
                RelaysView(),
                SizedBox(height: 16),
                BlossomsView(),
                // GetBuilder<SettingsController>(
                //   builder: (c) {
                //     return SwitchListTile(
                //       title: Text("Show advaced options"),
                //       value: c.showAdvancedOptions,
                //       onChanged: (value) {
                //         c.showAdvancedOptions = value;
                //       },
                //     );
                //   },
                // ),
                // TODO add select brightness
                // TODO add select theme color (default, device, profile picture, banner, custom)
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class UserView extends StatelessWidget {
  const UserView({super.key});

  @override
  Widget build(BuildContext context) {
    return FutureBuilder(
      future: DriveService().ndk.metadata.loadMetadata(
        DriveService().ndk.accounts.getPublicKey()!,
      ),
      builder: (context, snapshot) {
        final pubkey = DriveService().ndk.accounts.getPublicKey()!;
        String? pictureUrl;
        String? displayName;
        if (snapshot.data != null) {
          pictureUrl = snapshot.data!.picture;
          displayName = snapshot.data!.getName();
        }
        pictureUrl ??= "https://robohash.org/$pubkey";
        displayName ??= pubkeyToNpub(pubkey);

        return ListTile(
          leading: CircleAvatar(backgroundImage: NetworkImage(pictureUrl)),
          title: Text(
            displayName,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          trailing: IconButton(
            onPressed: SettingsController.to.logout,
            icon: Icon(Icons.logout),
          ),
        );
      },
    );
  }
}

class RelaysView extends StatelessWidget {
  const RelaysView({super.key});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16.0),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text("Relays", style: Theme.of(context).textTheme.titleLarge),
              TextButton(
                onPressed: () {
                  SettingsController.to.showDiscoverRelays =
                      !SettingsController.to.showDiscoverRelays;
                },
                child: Text("Discover"),
              ),
            ],
          ),
        ),
        GetBuilder<SettingsController>(
          builder: (c) {
            return Column(
              children: [
                ...c.relaysUrl.map((e) {
                  return ListTile(
                    title: Text(e),
                    trailing: OutlinedButton(
                      onPressed: () {
                        c.removeRelay(e);
                      },
                      child: Text("Remove"),
                    ),
                  );
                }),
              ],
            );
          },
        ),
        GetBuilder<SettingsController>(
          builder: (c) {
            return AnimatedSize(
              duration: Duration(milliseconds: 300),
              child: SizedBox(
                height: c.showDiscoverRelays ? null : 0,
                child: Column(
                  children: [
                    ...suggestedRelaysUrl
                        .where((url) => !c.relaysUrl.contains(url))
                        .map((e) {
                          return ListTile(
                            title: Text(e),
                            trailing: OutlinedButton(
                              onPressed: () {
                                c.addRelay(e);
                              },
                              child: Text("Add"),
                            ),
                          );
                        }),
                  ],
                ),
              ),
            );
          },
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16.0),
          child: TextField(
            controller: SettingsController.to.relayFieldController,
            decoration: InputDecoration(
              prefixIcon: Icon(Icons.language),
              labelText: "Nostr relay url",
              suffixIcon: TextButton(
                onPressed: SettingsController.to.addRelayFromField,
                child: Text("Connect"),
              ),
            ),
          ),
        ),
        SizedBox(height: 16),
      ],
    );
  }
}

class BlossomsView extends StatelessWidget {
  const BlossomsView({super.key});

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16.0),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                "Files storage locations",
                style: Theme.of(context).textTheme.titleLarge,
              ),
              TextButton(
                onPressed: () {
                  SettingsController.to.showDiscoverBlossomServers =
                      !SettingsController.to.showDiscoverBlossomServers;
                },
                child: Text("Discover"),
              ),
            ],
          ),
        ),
        GetBuilder<SettingsController>(
          builder: (c) {
            return Column(
              children: [
                ...c.blossomServersUrl.map((e) {
                  return ListTile(
                    title: Text(e),
                    trailing: OutlinedButton(
                      onPressed: () {
                        c.removeBlossomServer(e);
                      },
                      child: Text("Remove"),
                    ),
                  );
                }),
              ],
            );
          },
        ),
        GetBuilder<SettingsController>(
          builder: (c) {
            return AnimatedSize(
              duration: Duration(milliseconds: 300),
              child: SizedBox(
                height: c.showDiscoverBlossomServers ? null : 0,
                child: Column(
                  children: [
                    ...suggestedBlossomServersUrl
                        .where((url) => !c.blossomServersUrl.contains(url))
                        .map((e) {
                          return ListTile(
                            title: Text(e),
                            trailing: OutlinedButton(
                              onPressed: () {
                                c.addBlossomServer(e);
                              },
                              child: Text("Add"),
                            ),
                          );
                        }),
                  ],
                ),
              ),
            );
          },
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16.0),
          child: TextField(
            controller: SettingsController.to.blossomServerFieldController,
            decoration: InputDecoration(
              prefixIcon: Icon(Icons.language),
              labelText: "Blossom server url",
              suffixIcon: TextButton(
                onPressed: SettingsController.to.addBlossomServerFromField,
                child: Text("Connect"),
              ),
            ),
          ),
        ),
        SizedBox(height: 16),
      ],
    );
  }
}

class BootstrapRelaysView extends StatelessWidget {
  const BootstrapRelaysView({super.key});

  @override
  Widget build(BuildContext context) {
    return FutureBuilder(
      future: DriveService().ndk.userRelayLists.getSingleUserRelayList(
        DriveService().ndk.accounts.getPublicKey()!,
      ),
      builder: (context, snapshot) {
        if (snapshot.data == null) return Container();
        final relays = snapshot.data!.relays.keys;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.only(left: 16.0),
              child: Text(
                "Relays",
                style: Theme.of(context).textTheme.titleLarge,
              ),
            ),
            ...relays.map((e) {
              return ListTile(
                title: Text(e),
                trailing: OutlinedButton(
                  onPressed: () {},
                  child: Text("Remove"),
                ),
              );
            }),
            TextField(
              decoration: InputDecoration(
                prefixIcon: Icon(Icons.language),
                labelText: "Nostr relay url",
                suffixIcon: TextButton(
                  onPressed: () {},
                  child: Text("Connect"),
                ),
              ),
            ),
            SizedBox(height: 16),
          ],
        );
      },
    );
  }
}
