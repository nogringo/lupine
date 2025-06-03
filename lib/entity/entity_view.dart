// import 'package:flutter/material.dart';
// import 'package:get/get.dart';
// import 'package:intl/intl.dart';
// import 'package:lupine/entity/entity_controller.dart';
// import 'package:lupine/format_bytes.dart';
// import 'package:lupine/models/entity.dart';

// class EntityView extends StatelessWidget {
//   final Entity entity;

//   const EntityView({super.key, required this.entity});

//   @override
//   Widget build(BuildContext context) {
//     final entityController = EntityController();
//     Get.put(entityController);

//     Widget? preview;
//     if (entity.mime != null && entity.mime!.split("/")[0] == "image") {
//       preview = CircleAvatar(
//         child: FutureBuilder(
//           future: entity.download(),
//           builder: (context, snapshot) {
//             if (snapshot.data == null) {
//               return Container();
//             }
//             return Image.memory(snapshot.data!);
//           },
//         ),
//       );
//     }

//     return MouseRegion(
//       onEnter: (event) {
//         entityController.isHovered = true;
//       },
//       onExit: (event) {
//         entityController.isHovered = false;
//       },
//       child: GetBuilder<EntityController>(
//         init: entityController,
//         global: false,
//         builder: (entityController) {
//           return ColoredBox(
//             color: entityController.isHovered ? Theme.of(context).colorScheme.primaryContainer.withValues(alpha: .3) : Colors.transparent,
//             child: Padding(
//               padding: const EdgeInsets.symmetric(vertical: 4, horizontal: 12),
//               child: Row(
//                 children: [
//                   Expanded(flex: 3, child: Text(entity.name)),
//                   Expanded(child: Text(DateFormat().format(entity.createdAt))),
//                   Expanded(child: Text(formatBytes(entity.size))),
//                   Opacity(
//                     opacity: entityController.isHovered ? 1 : 0,
//                     child: IconButton(
//                       onPressed: () {},
//                       icon: Icon(Icons.more_vert),
//                     ),
//                   ),
//                 ],
//               ),
//             ),
//           );
//         },
//       ),
//     );
//   }
// }
