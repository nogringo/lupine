import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/loading/loading_controller.dart';

class LoadingPage extends StatelessWidget {
  const LoadingPage({super.key});

  @override
  Widget build(BuildContext context) {
    Get.put(LoadingController()..loadApp());
    return Scaffold(
      body: Center(
        child: CircularProgressIndicator(),
      ),
    );
  }
}