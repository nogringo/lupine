import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'package:encrypt/encrypt.dart';
import 'package:hex/hex.dart';
import 'package:crypto/crypto.dart';

class NsecEncryptor {
  static const ivLength = 16; // Taille de l'IV pour AES-CBC

  static Future<String> encryptString(String plaintext, String privkey) async {
    try {
      // Convertir nsec en clé de 32 bytes (256 bits) pour AES
      final keyBytes = _parseNsecKey(privkey);
      final key = Key(keyBytes);

      final iv = IV.fromSecureRandom(ivLength);

      // Créer l'encrypteur AES en mode CBC
      final encrypter = Encrypter(AES(key, mode: AESMode.cbc));

      // Chiffrer le texte
      final encrypted = encrypter.encrypt(plaintext, iv: iv);

      // Combiner IV + texte chiffré et encoder en base64
      final combined = Uint8List.fromList([...iv.bytes, ...encrypted.bytes]);
      return base64.encode(combined);
    } catch (e) {
      throw Exception('Erreur de chiffrement: $e');
    }
  }

  static Future<String> decryptString(
    String encryptedBase64,
    String privkey,
  ) async {
    try {
      // Convertir nsec en clé de 32 bytes (256 bits) pour AES
      final keyBytes = _parseNsecKey(privkey);
      final key = Key(keyBytes);

      // Décoder la base64
      final combined = base64.decode(encryptedBase64);

      final ivBytes = combined.sublist(0, ivLength);
      final iv = IV(ivBytes);

      // Extraire le texte chiffré (reste des bytes)
      final encryptedBytes = combined.sublist(ivLength);

      // Créer le déchiffreur AES en mode CBC
      final encrypter = Encrypter(AES(key, mode: AESMode.cbc));

      // Déchiffrer le texte
      final decrypted = encrypter.decrypt(Encrypted(encryptedBytes), iv: iv);

      return decrypted;
    } catch (e) {
      throw Exception('Erreur de déchiffrement: $e');
    }
  }

  /// Chiffre un fichier avec une clé nsec
  /// [inputPath]: Chemin du fichier original
  /// [privkey]: Clé secrète au format nsec (hexadécimal)
  /// [deterministic]: Si true, utilise un IV fixe pour un résultat déterministe
  static Future<Uint8List> encryptFile({
    required String inputPath,
    required String privkey,
    bool deterministic = false,
  }) async {
    // Convertir la clé nsec en bytes
    final keyBytes = _parseNsecKey(privkey);
    final key = Key(keyBytes);

    // Lire le fichier original
    final file = File(inputPath);
    final bytes = await file.readAsBytes();

    // Générer un IV (aléatoire ou fixe selon l'option)
    final iv =
        deterministic
            ? _generateDeterministicIV(key, bytes)
            : IV.fromSecureRandom(ivLength);

    // Initialiser l'encrypteur AES en mode CBC
    final encrypter = Encrypter(AES(key, mode: AESMode.cbc));

    // Chiffrer les données
    final encrypted = encrypter.encryptBytes(bytes, iv: iv);

    return Uint8List.fromList([...iv.bytes, ...encrypted.bytes]);
  }

  /// Déchiffre un fichier avec une clé nsec
  /// [inputPath]: Chemin du fichier chiffré (contient IV + données)
  /// [outputPath]: Chemin pour le fichier déchiffré
  /// [nsec]: Clé secrète au format nsec (hexadécimal)
  static Future<Uint8List> decryptFile({
    required Uint8List encryptedBytes,
    required String privkey,
  }) async {
    // Convertir la clé nsec en bytes
    final keyBytes = _parseNsecKey(privkey);
    final key = Key(keyBytes);

    if (encryptedBytes.length < ivLength) {
      throw Exception('Fichier chiffré invalide');
    }

    final iv = IV(encryptedBytes.sublist(0, ivLength));
    final data = encryptedBytes.sublist(ivLength);

    // Initialiser le déchiffreur AES en mode CBC
    final encrypter = Encrypter(AES(key, mode: AESMode.cbc));

    // Déchiffrer les données
    final decrypted = encrypter.decryptBytes(Encrypted(data), iv: iv);

    // Écrire le fichier déchiffré
    return Uint8List.fromList(decrypted);
  }

  /// Convertit une clé nsec en bytes pour le chiffrement
  static Uint8List _parseNsecKey(String privkey) {
    try {
      // Convertit l'hex en bytes
      final bytes = HEX.decode(privkey);

      // AES-256 nécessite une clé de 32 bytes
      if (bytes.length < 32) {
        throw Exception(
          'La clé nsec doit fournir au moins 32 bytes de données',
        );
      }

      // Prend les 32 premiers bytes (256 bits)
      return Uint8List.fromList(bytes.sublist(0, 32));
    } catch (e) {
      throw Exception('Format de clé nsec invalide: ${e.toString()}');
    }
  }

  /// Génère un IV déterministe à partir de la clé et du texte clair
  static IV _generateDeterministicIV(Key key, Uint8List plaintext) {
    final sha256s = sha256.convert([...key.bytes, ...plaintext]);
    return IV(Uint8List.fromList(sha256s.bytes.sublist(0, ivLength)));
  }
}
