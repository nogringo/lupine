import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/l10n/app_localizations.dart';
import 'package:lupine/screens/loading/loading_page.dart';
import 'package:lupine/repository.dart';

void main() {
  Get.put(Repository());
  runApp(const MainApp());
}

class MainApp extends StatelessWidget {
  const MainApp({super.key});

  @override
  Widget build(BuildContext context) {
    return GetMaterialApp(
      darkTheme: ThemeData.dark(),
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: LoadingPage(),
    );
  }
}
