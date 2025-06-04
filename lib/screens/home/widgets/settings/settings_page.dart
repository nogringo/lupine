import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/home/widgets/settings/settings_controller.dart';

class SettingsView extends StatelessWidget {
  const SettingsView({super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(SettingsController());
    return Center(
      child: FilledButton(
        onPressed: SettingsController.to.logout,
        child: Text("Logout"),
      ),
    );
  }
}
