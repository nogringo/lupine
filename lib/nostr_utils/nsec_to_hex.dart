import 'package:bech32/bech32.dart';

String nsecToHex(String nsec) {
  try {
    // Décoder le Bech32
    final bech32Data = Bech32Decoder().convert(nsec);
    
    // Convertir les 5-bit words en bytes (8-bit)
    final data = _convertBits(bech32Data.data, 5, 8, false);

    // Convertir les bytes en string hexadécimal
    return data.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
  } catch (e) {
    throw FormatException('Invalid nsec format: $e');
  }
}

/// Convertit une liste d'entiers d'un certain nombre de bits vers un autre
List<int> _convertBits(List<int> data, int from, int to, bool pad) {
  int acc = 0;
  int bits = 0;
  final maxv = (1 << to) - 1;
  final result = <int>[];

  for (final value in data) {
    if (value < 0 || (value >> from) != 0) {
      throw FormatException('Invalid value: $value');
    }

    acc = (acc << from) | value;
    bits += from;
    while (bits >= to) {
      bits -= to;
      result.add((acc >> bits) & maxv);
    }
  }

  if (pad) {
    if (bits > 0) {
      result.add((acc << (to - bits)) & maxv);
    }
  } else if (bits >= from || ((acc << (to - bits)) & maxv) != 0) {
    throw FormatException('Invalid padding');
  }

  return result;
}