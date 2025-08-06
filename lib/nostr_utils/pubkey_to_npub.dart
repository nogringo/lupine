import 'package:bech32/bech32.dart';

/// Convertit une liste de bits d'une taille à une autre (utile pour Bech32)
List<int> convertBits(List<int> data, int fromBits, int toBits, bool pad) {
  int acc = 0;
  int bits = 0;
  final result = <int>[];
  final maxv = (1 << toBits) - 1;

  for (final value in data) {
    if (value < 0 || (value >> fromBits) != 0) {
      throw ArgumentError('Invalid value: $value');
    }
    acc = (acc << fromBits) | value;
    bits += fromBits;
    while (bits >= toBits) {
      bits -= toBits;
      result.add((acc >> bits) & maxv);
    }
  }

  if (pad) {
    if (bits > 0) {
      result.add((acc << (toBits - bits)) & maxv);
    }
  } else if (bits >= fromBits || ((acc << (toBits - bits)) & maxv) != 0) {
    throw ArgumentError('Could not convert bits without padding');
  }

  return result;
}

/// Convertit une clé publique hexadécimale (64 caractères) en npub Bech32
String pubkeyToNpub(String hexPubkey) {
  if (!RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(hexPubkey)) {
    throw FormatException(
      'La clé publique doit être une chaîne hexadécimale de 64 caractères.',
    );
  }

  final bytes = List<int>.generate(
    32,
    (i) => int.parse(hexPubkey.substring(i * 2, i * 2 + 2), radix: 16),
  );

  final words = convertBits(bytes, 8, 5, true);
  final bech32 = Bech32('npub', words);
  return Bech32Codec().encode(bech32);
}
