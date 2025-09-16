// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Russian (`ru`).
class AppLocalizationsRu extends AppLocalizations {
  AppLocalizationsRu([String locale = 'ru']) : super(locale);

  @override
  String get helloWorld => 'Привет, мир!';

  @override
  String get settings => 'Настройки';

  @override
  String get switchAccount => 'Сменить аккаунт';

  @override
  String get switchOrAddAccount => 'Сменить или добавить аккаунт';

  @override
  String get addAnotherAccount => 'Добавить другой аккаунт';

  @override
  String get signOut => 'Выйти';
}
