import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/l10n/app_localizations.dart';
import 'package:lupine/screens/loading/loading_page.dart';
import 'package:lupine/repository.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter/foundation.dart' show defaultTargetPlatform, kIsWeb;

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await SystemTheme.accentColor.load();
  Get.put(Repository());
  runApp(const MainApp());
}

class MainApp extends StatelessWidget {
  const MainApp({super.key});

  @override
  Widget build(BuildContext context) {
    return SystemThemeBuilder(
      builder: (context, accent) {
        final supportAccentColor = defaultTargetPlatform.supportsAccentColor;
        Color accentColor =
            supportAccentColor ? accent.accent : accent.defaultAccentColor;
        if (kIsWeb) accentColor = const Color(0xFF6A5ACD);

        return GetMaterialApp(
          theme: ThemeData.from(
            colorScheme: ColorScheme.fromSeed(seedColor: accentColor),
          ),
          darkTheme: ThemeData.from(
            colorScheme: ColorScheme.fromSeed(
              seedColor: accentColor,
              brightness: Brightness.dark,
            ),
          ),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: LoadingPage(),
        );
      },
    );
  }
}
