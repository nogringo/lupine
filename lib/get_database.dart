import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';
import 'package:sembast_web/sembast_web.dart';
import 'package:sembast/sembast_io.dart';
import 'package:path/path.dart' as p;

Future<Database> getDatabase([String dbName = "ndk_cache_manager"]) async {
  final name = kDebugMode ? '${dbName}_dev' : dbName;

  if (kIsWeb) {
    var factory = databaseFactoryWeb;
    return await factory.openDatabase(name);
  }

  final Directory appSupportDir = await getApplicationSupportDirectory();
  return await databaseFactoryIo.openDatabase(
    p.join(appSupportDir.path, '$name.db'),
  );
}
