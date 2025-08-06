import 'dart:typed_data';
import 'package:pointycastle/ecc/api.dart';
import 'package:pointycastle/ecc/curves/secp256k1.dart';

/// Converts a hexadecimal string to a Uint8List.
Uint8List hexToBytes(String hexString) {
  if (hexString.length % 2 != 0) {
    throw FormatException('Odd length hex string');
  }
  final List<int> bytes = [];
  for (int i = 0; i < hexString.length; i += 2) {
    final String byteHex = hexString.substring(i, i + 2);
    bytes.add(int.parse(byteHex, radix: 16));
  }
  return Uint8List.fromList(bytes);
}

/// Converts a Uint8List to a hexadecimal string.
String bytesToHex(Uint8List bytes) {
  return bytes.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
}

/// Converts a Uint8List to a BigInt.
BigInt decodeBigInt(Uint8List bytes) {
  BigInt result = BigInt.zero;
  for (int i = 0; i < bytes.length; i++) {
    result = result + (BigInt.from(bytes[i]) << (8 * (bytes.length - 1 - i)));
  }
  return result;
}

/// Helper function to encode a BigInt to exactly 32 bytes (for X/Y coordinates)
Uint8List _encodeBigIntTo32Bytes(BigInt bigInt) {
  final List<int> bytes = List<int>.filled(32, 0);
  int i = 31;
  BigInt temp = bigInt;
  while (temp > BigInt.zero && i >= 0) {
    bytes[i--] = (temp & BigInt.from(0xFF)).toInt();
    temp = temp >> 8;
  }
  return Uint8List.fromList(bytes);
}

/// Derives the Nostr public key (X-coordinate only) from a given Nostr private key.
///
/// [privateKeyHex] should be a hexadecimal string representation of the
/// 32-byte private key.
/// Returns a hexadecimal string representing only the X-coordinate of the public key (32 bytes),
/// which is the standard format for Nostr pubkeys.
String privkeyToPubkey(String privateKeyHex) {
  final Uint8List privateKeyBytes = hexToBytes(privateKeyHex);

  if (privateKeyBytes.length != 32) {
    throw ArgumentError(
      'Nostr private key must be 32 bytes long after decoding from hex.',
    );
  }

  // Nostr uses the SECP256k1 elliptic curve
  final ECDomainParameters ecParams = ECCurve_secp256k1();
  final BigInt privateKeyInt = decodeBigInt(privateKeyBytes);
  final ECPrivateKey privateKey = ECPrivateKey(privateKeyInt, ecParams);

  // Derive the public key point (P = d * G)
  final ECPoint publicKeyPoint = (ecParams.G * privateKey.d)!;

  // Get the X-coordinate as BigInt
  final BigInt? xCoordinate = publicKeyPoint.x!.toBigInteger();

  // Convert BigInt X-coordinate to exactly 32 bytes, as required for Nostr pubkeys
  final Uint8List xCoordinateBytes = _encodeBigIntTo32Bytes(xCoordinate!);

  // Convert the X-coordinate bytes back to a hex string
  return bytesToHex(xCoordinateBytes);
}
