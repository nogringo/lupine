import 'package:ndk/ndk.dart';

class NoEventVerifier extends EventVerifier {
  @override
  Future<bool> verify(Nip01Event event) async {
    return true;
  }
}
