import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/l10n/app_localizations.dart';
import 'package:lupine/middlewares/router_login_middleware.dart';
import 'package:lupine/screens/home/home_page.dart';
import 'package:lupine/screens/loading/loading_page.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/login/login_page.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter/foundation.dart';
import 'package:toastification/toastification.dart';
import 'package:window_manager/window_manager.dart';
import 'package:url_strategy/url_strategy.dart';

void main() async {
  setPathUrlStrategy();

  WidgetsFlutterBinding.ensureInitialized();
  await SystemTheme.accentColor.load();

  if (isDesktop) {
    await windowManager.ensureInitialized();
    windowManager.waitUntilReadyToShow(
      WindowOptions(titleBarStyle: TitleBarStyle.hidden),
    );
  }

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

        ThemeData getTheme([Brightness? brightness]) {
          brightness = brightness ?? Brightness.light;
          final bool isLightTheme = brightness == Brightness.light;

          final colorScheme = ColorScheme.fromSeed(
            seedColor: accentColor,
            brightness: brightness,
          );

          return ThemeData(
            appBarTheme: AppBarTheme(
              systemOverlayStyle: SystemUiOverlayStyle(
                statusBarBrightness:
                    isLightTheme
                        ? Brightness.dark
                        : Brightness.light, //! It does not switch on emulator
                systemNavigationBarColor: colorScheme.surfaceContainer,
                systemNavigationBarIconBrightness:
                    isLightTheme ? Brightness.dark : Brightness.light,
              ),
            ),
            colorScheme: colorScheme,
            brightness: brightness,
          );
        }

        return ToastificationWrapper(
          child: GetMaterialApp(
            theme: getTheme(),
            darkTheme: getTheme(Brightness.dark),
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            getPages: [
              GetPage(
                name: AppRoutes.home,
                page: () => const HomePage(),
                middlewares: [RouterLoginMiddleware()],
              ),
              GetPage(name: AppRoutes.login, page: () => const LoginPage()),
              GetPage(name: AppRoutes.loading, page: () => const LoadingPage()),
            ],
          ),
        );
      },
    );
  }
}
