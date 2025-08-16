import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/config.dart';
import 'package:lupine/repository.dart';
import 'package:lupine/screens/home/widgets/explorer_view.dart';

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
      icon: Icons.delete_outlined,
      selectedIcon: Icons.delete,
      label: 'Trash',
      view: ExplorerView(),
      onSelected: () {
        Repository.to.fileExplorerViewPath = trashPath;
      },
    ),
  ];

  RxBool expandedRail = false.obs;

  bool _showFABShowOptions = false;
  int _selectedIndex = 0;

  bool get showFABShowOptions => _showFABShowOptions;
  set showFABShowOptions(bool value) {
    _showFABShowOptions = value;
    update();
  }

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

  void toggleFABOptions() {
    showFABShowOptions = !showFABShowOptions;
  }

  void closeFABOptions() {
    showFABShowOptions = false;
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
