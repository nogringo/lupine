// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Spanish Castilian (`es`).
class AppLocalizationsEs extends AppLocalizations {
  AppLocalizationsEs([String locale = 'es']) : super(locale);

  @override
  String get helloWorld => '¡Hola, mundo!';

  @override
  String get settings => 'Configuración';

  @override
  String get switchAccount => 'Cambiar cuenta';

  @override
  String get switchOrAddAccount => 'Cambiar o añadir cuenta';

  @override
  String get addAnotherAccount => 'Añadir otra cuenta';

  @override
  String get signOut => 'Cerrar sesión';
}
