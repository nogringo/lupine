import 'dart:io';

import 'package:flutter/foundation.dart';

final isDesktop =
    !kIsWeb && (Platform.isWindows || Platform.isLinux || Platform.isMacOS);
