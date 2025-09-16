// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Japanese (`ja`).
class AppLocalizationsJa extends AppLocalizations {
  AppLocalizationsJa([String locale = 'ja']) : super(locale);

  @override
  String get helloWorld => 'こんにちは、世界！';

  @override
  String get settings => '設定';

  @override
  String get switchAccount => 'アカウントを切り替える';

  @override
  String get switchOrAddAccount => 'アカウントを切り替えるか追加する';

  @override
  String get addAnotherAccount => '別のアカウントを追加';

  @override
  String get signOut => 'ログアウト';
}
