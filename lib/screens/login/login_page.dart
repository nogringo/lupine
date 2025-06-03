import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/screens/login/login_controller.dart';

class LoginPage extends StatelessWidget {
  const LoginPage({super.key});

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: 300),
          child: GetBuilder(
            init: LoginController(),
            builder: (loginController) {
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(height: 16),
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
