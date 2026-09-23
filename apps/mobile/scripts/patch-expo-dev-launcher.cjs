const fs = require('node:fs');
const path = require('node:path');

const file = path.join(
  path.dirname(require.resolve('expo-dev-launcher/package.json')),
  'android/src/debug/java/expo/modules/devlauncher/DevLauncherController.kt',
);
const before = 'intent.categories?.let {\n            categories.addAll(it)\n          }';
const after = 'intent.categories?.let { categories ->\n            categories.forEach { addCategory(it) }\n          }';
const source = fs.readFileSync(file, 'utf8');

if (!source.includes(after)) {
  if (!source.includes(before)) {
    throw new Error('Unexpected Expo dev launcher source; review the Android deep link patch.');
  }
  fs.writeFileSync(file, source.replace(before, after));
}
