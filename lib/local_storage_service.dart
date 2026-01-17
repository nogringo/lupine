import 'package:get/get.dart';
import 'package:lupine/get_database.dart';
import 'package:sembast/sembast.dart';

class LocalStorageService extends GetxService {
  static LocalStorageService get to => Get.find();

  late Database _db;
  final _store = StoreRef<String, dynamic>.main();
  final Map<String, dynamic> _cache = {};

  Future<LocalStorageService> init() async {
    _db = await getDatabase("lupine_prefs");
    final records = await _store.find(_db);
    for (final record in records) {
      _cache[record.key] = record.value;
    }
    return this;
  }

  T? read<T>(String key) {
    return _cache[key] as T?;
  }

  void write(String key, dynamic value) {
    _cache[key] = value;
    _store.record(key).put(_db, value);
  }

  void remove(String key) {
    _cache.remove(key);
    _store.record(key).delete(_db);
  }
}
