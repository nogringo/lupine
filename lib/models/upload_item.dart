import 'dart:typed_data';

enum UploadStatus { waiting, uploading, completed, failed }

class UploadItem {
  final String id;
  final String fileName;
  final String destPath;
  final int fileSize;
  final Uint8List fileData;
  final String? mimeType;
  UploadStatus status;
  String? errorMessage;
  final DateTime addedAt;
  DateTime? completedAt;

  UploadItem({
    required this.id,
    required this.fileName,
    required this.destPath,
    required this.fileSize,
    required this.fileData,
    this.mimeType,
    this.status = UploadStatus.waiting,
    this.errorMessage,
    DateTime? addedAt,
    this.completedAt,
  }) : addedAt = addedAt ?? DateTime.now();

  UploadItem copyWith({
    String? id,
    String? fileName,
    String? destPath,
    int? fileSize,
    Uint8List? fileData,
    String? mimeType,
    UploadStatus? status,
    String? errorMessage,
    DateTime? addedAt,
    DateTime? completedAt,
  }) {
    return UploadItem(
      id: id ?? this.id,
      fileName: fileName ?? this.fileName,
      destPath: destPath ?? this.destPath,
      fileSize: fileSize ?? this.fileSize,
      fileData: fileData ?? this.fileData,
      mimeType: mimeType ?? this.mimeType,
      status: status ?? this.status,
      errorMessage: errorMessage ?? this.errorMessage,
      addedAt: addedAt ?? this.addedAt,
      completedAt: completedAt ?? this.completedAt,
    );
  }
}
