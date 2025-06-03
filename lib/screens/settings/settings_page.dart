import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/settings/settings_controller.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(SettingsController());
    return Scaffold(
      appBar: AppBar(title: Text("Settings")),
      body: Center(
        child: FilledButton(onPressed: SettingsController.to.logout, child: Text("Logout")),
      ),
    );
  }
}
