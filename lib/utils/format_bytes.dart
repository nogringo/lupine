import 'dart:math';

String formatBytes(int bytes, {int decimals = 1}) {
  if (bytes <= 0) return "0 octet";

  const suffixes = ["octets", "Ko", "Mo", "Go", "To", "Po"];
  final i = (log(bytes) / log(1024)).floor();
  final value = bytes / pow(1024, i);

  // Formate le nombre avec le bon nombre de décimales
  final formattedValue = value.toStringAsFixed(decimals);

  // Enlève les .00 si c'est un nombre entier
  final cleanValue =
      formattedValue.endsWith('.00')
          ? formattedValue.substring(0, formattedValue.length - 3)
          : formattedValue.replaceAll(RegExp(r'\.?0*$'), '');

  return '$cleanValue ${suffixes[i]}';
}
