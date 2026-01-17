import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/local_storage_service.dart';
import 'package:lupine/config.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/controllers/upload_queue_controller.dart';
import 'package:lupine/l10n/app_localizations.dart';
import 'package:lupine/middlewares/router_login_middleware.dart';
import 'package:lupine/screens/home/home_page.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/login/login_page.dart';
import 'package:lupine/screens/share/share_page.dart';
import 'package:lupine/screens/switch_account/switch_account_page.dart';
import 'package:lupine/screens/user_profile/user_profile_page.dart';
import 'package:nostr_widgets/functions/functions.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter/foundation.dart';
import 'package:toastification/toastification.dart';
import 'package:window_manager/window_manager.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:nostr_widgets/l10n/app_localizations.dart' as nostr_widgets;

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await Get.putAsync(() => LocalStorageService().init());

  await SystemTheme.accentColor.load();

  if (isDesktop) {
    await windowManager.ensureInitialized();
    windowManager.waitUntilReadyToShow(
      WindowOptions(titleBarStyle: TitleBarStyle.hidden),
    );
  }

  final repository = Repository();
  Get.put(repository);
  await repository.init();

  Get.put(UploadQueueController());

  await nRestoreAccounts(repository.ndk);

  if (repository.ndk.accounts.isLoggedIn) {
    await repository.driveService.onAccountChanged();
  }

  runApp(const MainApp());
}

class MainApp extends StatelessWidget {
  const MainApp({super.key});

  @override
  Widget build(BuildContext context) {
    return SystemThemeBuilder(
      builder: (context, accent) {
        final supportAccentColor = defaultTargetPlatform.supportsAccentColor;
        Color accentColor = supportAccentColor
            ? accent.accent
            : accent.defaultAccentColor;
        if (kIsWeb) accentColor = Colors.teal;

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
                statusBarBrightness: isLightTheme
                    ? Brightness.dark
                    : Brightness.light, //! It does not switch on emulator
                systemNavigationBarColor: colorScheme.surfaceContainer,
                systemNavigationBarIconBrightness: isLightTheme
                    ? Brightness.dark
                    : Brightness.light,
              ),
            ),
            colorScheme: colorScheme,
            brightness: brightness,
          );
        }

        return ToastificationWrapper(
          child: GetMaterialApp(
            title: appTitle,
            theme: getTheme(),
            darkTheme: getTheme(Brightness.dark),
            localizationsDelegates: [
              AppLocalizations.delegate,
              GlobalMaterialLocalizations.delegate,
              GlobalWidgetsLocalizations.delegate,
              GlobalCupertinoLocalizations.delegate,
              nostr_widgets.AppLocalizations.delegate,
            ],
            supportedLocales: AppLocalizations.supportedLocales,
            getPages: [
              GetPage(
                name: AppRoutes.home,
                page: () => const HomePage(),
                middlewares: [RouterLoginMiddleware()],
              ),
              GetPage(name: AppRoutes.login, page: () => const LoginPage()),
              GetPage(
                name: AppRoutes.userProfile,
                page: () => const UserProfilePage(),
                middlewares: [RouterLoginMiddleware()],
              ),
              GetPage(
                name: AppRoutes.switchAccount,
                page: () => const SwitchAccountPage(),
                middlewares: [RouterLoginMiddleware()],
              ),
              GetPage(
                name: AppRoutes.share,
                page: () => const SharePage(),
                middlewares: [],
              ),
            ],
          ),
        );
      },
    );
  }
}
