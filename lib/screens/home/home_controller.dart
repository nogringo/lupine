import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/explorer_view.dart';
import 'package:lupine/screens/home/widgets/photos_view.dart';
import 'package:lupine/screens/home/widgets/settings/settings_page.dart';

class HomeController extends GetxController {
  static HomeController get to => Get.find();

  static List<Destination> destinations = [
    Destination(
      icon: Icons.home_outlined,
      selectedIcon: Icons.home,
      label: 'Files',
      view: ExplorerView(),
      onSelected: () {
        Repository.to.fileExplorerViewPath = myFilesPath;
      },
    ),
    Destination(
      icon: Icons.photo_outlined,
      selectedIcon: Icons.photo,
      label: 'Photos',
      view: PhotosView(),
    ),
    Destination(
      icon: Icons.delete_outlined,
      selectedIcon: Icons.delete,
      label: 'Trash',
      view: ExplorerView(),
      onSelected: () {
        Repository.to.fileExplorerViewPath = trashPath;
      },
    ),
    Destination(
      icon: Icons.settings_outlined,
      selectedIcon: Icons.settings,
      label: 'Settings',
      view: SettingsView(),
    ),
  ];

  int _selectedIndex = 0;

  int get selectedIndex => _selectedIndex;
  set selectedIndex(int value) {
    _selectedIndex = value;

    final onSelected = destinations[value].onSelected;
    if (onSelected != null) onSelected();

    update();
  }

  void onDestinationSelected(int value) {
    selectedIndex = value;
  }
}

class Destination {
  final IconData icon;
  final IconData selectedIcon;
  final String label;
  final Widget view;
  final Function? onSelected;

  const Destination({
    required this.icon,
    required this.selectedIcon,
    required this.label,
    required this.view,
    this.onSelected,
  });
}
