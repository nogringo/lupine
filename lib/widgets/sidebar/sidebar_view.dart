import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/widgets/sidebar/sidebar_controller.dart';

class SidebarView extends StatelessWidget {
  const SidebarView({super.key});

  @override
  Widget build(BuildContext context) {
    return GetBuilder(
      init: SidebarController(),
      builder: (sidebarController) {
        return AnimatedContainer(
          duration: Duration(milliseconds: 300),
          width: sidebarController.extended ? 200 : 80,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (sidebarController.extended)
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: FilledButton.icon(
                    onPressed: () {},
                    label: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 10),
                      child: Text("New"),
                    ),
                    icon: Icon(Icons.add),
                  ),
                ),
              if (!sidebarController.extended)
                IconButton.filled(onPressed: () {}, icon: Icon(Icons.add)),
              SizedBox(height: 4),
              if (sidebarController.extended)
                ListTile(
                  leading: Icon(Icons.home_rounded),
                  title: Text(
                    "My files",
                    overflow: TextOverflow.fade,
                    maxLines: 1,
                  ),
                  onTap: () {
                    sidebarController.goTo(myFilesPath);
                  },
                ),
              if (!sidebarController.extended)
                IconButton(
                  onPressed: () {
                    sidebarController.goTo(myFilesPath);
                  },
                  icon: Icon(Icons.home_rounded),
                ),
              if (!sidebarController.extended) SizedBox(height: 4),
              if (sidebarController.extended)
                ListTile(
                  leading: Icon(Icons.delete),
                  title: Text(
                    "Trash",
                    overflow: TextOverflow.fade,
                    maxLines: 1,
                  ),
                  onTap: () {
                    sidebarController.goTo(trashPath);
                  },
                ),
              if (!sidebarController.extended)
                IconButton(
                  onPressed: () {
                    sidebarController.goTo(trashPath);
                  },
                  icon: Icon(Icons.delete),
                ),
              Expanded(child: Container()),
              Padding(
                padding: const EdgeInsets.all(8.0),
                child: Align(
                  alignment:
                      sidebarController.extended
                          ? Alignment.centerRight
                          : Alignment.center,
                  child: IconButton(
                    onPressed: sidebarController.toogleExtend,
                    icon: AnimatedRotation(
                      duration: Duration(milliseconds: 100),
                      turns: sidebarController.extended ? .5 : 1,
                      child: Icon(Icons.arrow_forward_ios_rounded),
                    ),
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
