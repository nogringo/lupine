import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/login/login_controller.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:ndk/domain_layer/usecases/bunkers/models/nostr_connect.dart';
import 'package:nostr_widgets/nostr_widgets.dart';
import 'package:window_manager/window_manager.dart';

class LoginPage extends StatelessWidget {
  const LoginPage({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = Get.put(LoginController());

    return Scaffold(
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        surfaceTintColor: Colors.transparent,
        title: () {
          final title = Align(
            alignment: AlignmentDirectional.centerStart,
            child: Text(""),
          );
          if (isDesktop) return DragToMoveArea(child: title);
          return title;
        }(),
        actions: [
          if (isDesktop)
            Align(alignment: Alignment.topCenter, child: WindowButtons()),
        ],
      ),
      body: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: 380),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                "Lupine sign in",
                style: Theme.of(context).textTheme.displaySmall,
              ),
              SizedBox(height: 32),
              Obx(
                () =>
                    controller.isLoading.value
                        ? Center(child: CircularProgressIndicator())
                        : NLogin(
                          ndk: Repository.to.ndk,
                          enablePubkeyLogin: false,
                          nostrConnect: NostrConnect(
                            relays: ["wss://relay.nsec.app"],
                            appName: appTitle,
                          ),
                          onLoggedIn: controller.onLoggedIn,
                        ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
