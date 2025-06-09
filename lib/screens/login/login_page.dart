import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/constants.dart';
import 'package:lupine/screens/login/login_controller.dart';
import 'package:lupine/widgets/desktop/window_buttons.dart';
import 'package:window_manager/window_manager.dart';

class LoginPage extends StatelessWidget {
  const LoginPage({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        surfaceTintColor: Colors.transparent,
        title: () {
          final title = Align(
            alignment: AlignmentDirectional.centerStart,
            child: Text(""),
          );
          if (isDesktop) return DragToMoveArea(child: title);
          return title;
        }(),
        actions: [
          if (isDesktop)
            Align(alignment: Alignment.topCenter, child: WindowButtons()),
        ],
      ),
      body: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: 300),
          child: GetBuilder(
            init: LoginController(),
            builder: (loginController) {
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    "Lupine login",
                    style: Theme.of(context).textTheme.displayMedium,
                  ),
                  SizedBox(height: 16),
                  Text(
                    "Your nsec",
                    style: Theme.of(context).textTheme.labelLarge,
                  ),
                  TextField(
                    controller: loginController.nsecController,
                    decoration: InputDecoration(hintText: "nsec..."),
                    onChanged: loginController.nsecChanged,
                  ),
                  SizedBox(height: 16),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}
