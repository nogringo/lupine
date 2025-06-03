import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/widgets/create_folder_dialog.dart';
import 'package:lupine/file_explorer/file_explorer_view.dart';
import 'package:lupine/repository.dart';
import 'package:flutter_gen/gen_l10n/app_localizations.dart';
import 'package:lupine/widgets/sidebar/sidebar_view.dart';

void main() {
  Get.put(Repository());
  runApp(const MainApp());
}

class MainApp extends StatelessWidget {
  const MainApp({super.key});

  @override
  Widget build(BuildContext context) {
    return GetMaterialApp(
      darkTheme: ThemeData.dark(),
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Builder(
        builder: (context) {
          return DropTarget(
            onDragDone: (detail) {
              for (var file in detail.files) {
                final entity = FileSystemEntity.typeSync(file.path);

                if (entity == FileSystemEntityType.directory) {
                  Repository.to.addFolder(
                    file.path,
                    Repository.to.fileExplorerViewPath,
                  );
                } else if (entity == FileSystemEntityType.file) {
                  Repository.to.addFile(file.path);
                }
              }
            },
            onDragEntered: (detail) {
              Repository.to.isDraggingFile = true;
            },
            onDragExited: (detail) {
              Repository.to.isDraggingFile = false;
            },
            child: GetBuilder(
              init: Repository.to,
              builder: (repository) {
                if (repository.isDraggingFile) {
                  return Scaffold(
                    body: Center(
                      child: Text(
                        "Drop here",
                        style: Theme.of(context).textTheme.displayLarge,
                      ),
                    ),
                  );
                }
                return Scaffold(
                  appBar: AppBar(
                    backgroundColor: Colors.transparent,
                    surfaceTintColor: Colors.transparent,
                    title: Text("Lupine"),
                    actions: [
                      IconButton(
                        onPressed: () async {
                          repository.listenEvents();
                        },
                        icon: Icon(Icons.list),
                      ),
                    ],
                  ),
                  body: Row(
                    children: [
                      SidebarView(),
                      Expanded(
                        child: Container(
                          margin: EdgeInsets.only(right: 8, bottom: 8),
                          decoration: BoxDecoration(
                            color:
                                Theme.of(context).colorScheme.surfaceContainerLow,
                            borderRadius: BorderRadius.circular(16),
                          ),
                          child: Column(
                            children: [
                              Padding(
                                padding: const EdgeInsets.all(8.0),
                                child: Row(
                                  children: [
                                    Expanded(child: FileExplorerPathView()),
                                    IconButton(
                                      onPressed: () {
                                        Get.dialog(CreateFolderDialog());
                                      },
                                      icon: Icon(
                                        Icons.create_new_folder_outlined,
                                      ),
                                    ),
                                    IconButton(
                                      onPressed: repository.pickFolder,
                                      icon: Icon(
                                        Icons.drive_folder_upload_outlined,
                                      ),
                                    ),
                                    IconButton(
                                      onPressed: repository.pickFiles,
                                      icon: Icon(Icons.upload_file),
                                    ),
                                  ],
                                ),
                              ),
                              Expanded(
                                child: SingleChildScrollView(
                                  child: FileExplorerView(),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ],
                  ),
                );
              },
            ),
          );
        },
      ),
    );
  }
}

class FileExplorerPathView extends StatelessWidget {
  const FileExplorerPathView({super.key});

  @override
  Widget build(BuildContext context) {
    List<Widget> children = [];

    final buttonList = Repository.to.fileExplorerViewPath.split("/");

    for (var i = 0; i < buttonList.length; i++) {
      final e = buttonList[i];

      if (e == "") continue;

      children.addAll([
        TextButton(
          onPressed: () {
            Repository.to.fileExplorerViewPath = buttonList
                .sublist(0, i + 1)
                .join("/");
          },
          child: Text(e),
        ),
        Icon(Icons.chevron_right_rounded),
      ]);
    }

    children.removeLast();

    return Row(children: children);
  }
}
