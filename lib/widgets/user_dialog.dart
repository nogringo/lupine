import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:lupine/app_routes.dart';
import 'package:lupine/l10n/app_localizations.dart';
import 'package:lupine/repository.dart';
import 'package:nostr_widgets/nostr_widgets.dart';

void showUserDialog(BuildContext context) {
  final RenderBox button = context.findRenderObject() as RenderBox;
  final RenderBox overlay =
      Navigator.of(context).overlay!.context.findRenderObject() as RenderBox;
  final RelativeRect position = RelativeRect.fromRect(
    Rect.fromPoints(
      button.localToGlobal(Offset.zero, ancestor: overlay),
      button.localToGlobal(
        button.size.bottomRight(Offset.zero),
        ancestor: overlay,
      ),
    ),
    Offset.zero & overlay.size,
  );

  showDialog(
    context: context,
    barrierColor: Colors.transparent,
    builder:
        (context) => Stack(
          children: [
            Positioned.fill(
              child: GestureDetector(
                onTap: () => Navigator.of(context).pop(),
                child: Container(color: Colors.transparent),
              ),
            ),
            Positioned(
              top: position.top + kToolbarHeight,
              right: 8,
              child: Material(
                elevation: 8,
                borderRadius: BorderRadius.circular(32),
                child: Container(
                  constraints: BoxConstraints(maxWidth: 320),
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.surface,
                    borderRadius: BorderRadius.circular(32),
                    border: Border.all(
                      color: Theme.of(context).colorScheme.outlineVariant,
                      width: 2,
                    ),
                  ),
                  padding: EdgeInsets.all(20),
                  child: _UserDialogContent(),
                ),
              ),
            ),
          ],
        ),
  );
}

class _UserDialogContent extends StatelessWidget {
  const _UserDialogContent();

  @override
  Widget build(BuildContext context) {
    final ndk = Repository.to.ndk;
    final currentPubkey = ndk.accounts.getPublicKey();
    final hasMultipleAccounts = ndk.accounts.accounts.keys.length > 1;

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Center(
          child: NPicture(
            ndk: ndk,
            pubkey: currentPubkey,
            circleAvatarRadius: 40,
          ),
        ),
        SizedBox(height: 8),
        Center(
          child: NName(
            ndk: ndk,
            pubkey: currentPubkey,
            style: Theme.of(context).textTheme.titleLarge,
          ),
        ),
        SizedBox(height: 32),
        OutlinedButton(
          onPressed: () {
            Navigator.pop(context);
            Get.toNamed(AppRoutes.userProfile);
          },
          child: Text(AppLocalizations.of(context)!.settings),
        ),
        SizedBox(height: 8),
        OutlinedButton(
          onPressed: () {
            Navigator.pop(context);
            if (hasMultipleAccounts) {
              Get.toNamed(AppRoutes.switchAccount);
            } else {
              Get.toNamed(AppRoutes.login);
            }
          },
          child: Text(
            hasMultipleAccounts
                ? AppLocalizations.of(context)!.switchOrAddAccount
                : AppLocalizations.of(context)!.addAnotherAccount,
          ),
        ),
        SizedBox(height: 8),
        OutlinedButton(
          onPressed: () {
            Navigator.pop(context);
            Repository.to.logout();
          },
          child: Text(AppLocalizations.of(context)!.signOut),
        ),
      ],
    );
  }
}
