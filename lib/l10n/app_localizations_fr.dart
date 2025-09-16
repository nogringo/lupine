// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for French (`fr`).
class AppLocalizationsFr extends AppLocalizations {
  AppLocalizationsFr([String locale = 'fr']) : super(locale);

  @override
  String get helloWorld => 'Bonjour monde !';

  @override
  String get settings => 'Paramètres';

  @override
  String get switchAccount => 'Changer de compte';

  @override
  String get switchOrAddAccount => 'Changer ou ajouter un compte';

  @override
  String get addAnotherAccount => 'Ajouter un autre compte';

  @override
  String get signOut => 'Se déconnecter';
}
