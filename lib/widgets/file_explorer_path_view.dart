import 'package:flutter/material.dart';
import 'package:lupine/repository.dart';

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