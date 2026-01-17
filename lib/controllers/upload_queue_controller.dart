import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:get/get.dart';
import 'package:get_storage/get_storage.dart';
import 'package:lupine/config.dart';
import 'package:lupine/models/upload_item.dart';
import 'package:lupine/repository.dart';

class UploadQueueController extends GetxController {
  static UploadQueueController get to => Get.find();

  final _storage = GetStorage(appTitle);
  final List<UploadItem> _queue = [];
  bool _isProcessing = false;

  late bool _isExpanded = _storage.read('uploadPanelExpanded') ?? true;

  List<UploadItem> get queue => _queue;

  bool get isExpanded => _isExpanded;
  set isExpanded(bool value) {
    _isExpanded = value;
    _storage.write('uploadPanelExpanded', value);
    update();
  }

  bool get hasItems => _queue.isNotEmpty;

  int get totalCount => _queue.length;

  int get completedCount =>
      _queue.where((item) => item.status == UploadStatus.completed).length;

  int get uploadingCount =>
      _queue.where((item) => item.status == UploadStatus.uploading).length;

  int get waitingCount =>
      _queue.where((item) => item.status == UploadStatus.waiting).length;

  int get failedCount =>
      _queue.where((item) => item.status == UploadStatus.failed).length;

  bool get isAllCompleted =>
      _queue.isNotEmpty &&
      _queue.every(
        (item) =>
            item.status == UploadStatus.completed ||
            item.status == UploadStatus.failed,
      );

  String _generateId() {
    final timestamp = DateTime.now().microsecondsSinceEpoch;
    final random = Random().nextInt(999999);
    return '$timestamp$random';
  }

  void enqueue({
    required String fileName,
    required String destPath,
    required Uint8List fileData,
    String? mimeType,
  }) {
    final item = UploadItem(
      id: _generateId(),
      fileName: fileName,
      destPath: destPath,
      fileSize: fileData.length,
      fileData: fileData,
      mimeType: mimeType,
      status: UploadStatus.waiting,
    );

    _queue.add(item);
    update();
    _processQueue();
  }

  Future<void> _processQueue() async {
    if (_isProcessing) return;

    final nextItem = _queue.cast<UploadItem?>().firstWhere(
      (item) => item?.status == UploadStatus.waiting,
      orElse: () => null,
    );

    if (nextItem == null) return;

    _isProcessing = true;
    nextItem.status = UploadStatus.uploading;
    update();

    try {
      await Repository.to.driveService.uploadFile(
        fileData: nextItem.fileData,
        path: nextItem.destPath,
        fileType: nextItem.mimeType,
      );

      nextItem.status = UploadStatus.completed;
      nextItem.completedAt = DateTime.now();
    } catch (e) {
      nextItem.status = UploadStatus.failed;
      nextItem.errorMessage = e.toString();
      if (kDebugMode) {
        print('Upload failed for ${nextItem.fileName}: $e');
      }
    }

    _isProcessing = false;
    update();

    _processQueue();
  }

  void retry(String id) {
    final item = _queue.firstWhere((item) => item.id == id);
    item.status = UploadStatus.waiting;
    item.errorMessage = null;
    update();
    _processQueue();
  }

  void removeItem(String id) {
    _queue.removeWhere((item) => item.id == id);
    update();
  }

  void clearCompleted() {
    _queue.removeWhere(
      (item) =>
          item.status == UploadStatus.completed ||
          item.status == UploadStatus.failed,
    );
    update();
  }

  void clearAll() {
    _queue.clear();
    update();
  }
}
