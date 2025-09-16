// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Chinese (`zh`).
class AppLocalizationsZh extends AppLocalizations {
  AppLocalizationsZh([String locale = 'zh']) : super(locale);

  @override
  String get helloWorld => '你好，世界！';

  @override
  String get settings => '设置';

  @override
  String get switchAccount => '切换账户';

  @override
  String get switchOrAddAccount => '切换或添加账户';

  @override
  String get addAnotherAccount => '添加另一个账户';

  @override
  String get signOut => '退出登录';
}
