# Kratos mobile

Expo implementation of the GPUI application, with Android as the primary target.

The GPUI app remains the product reference. The current SwiftUI client is a
transport/protocol reference only; it does not define this app's feature scope.

See [the parity inventory](docs/gpui-parity.md) before adding a surface.

## Development

Use a supported Node release (20.19.4+, 22.13+, 24.3+, or 25+), then:

```sh
source Native/env.sh
npm install
npm run android
```

Tailcat is the app's connection method. Pair with a `kratos-pair:` invitation
from the Kratos desktop, or reconnect a saved pairing. The app then uses
authenticated ControlRpc through Tailcat. Expo Go cannot load the native module;
use a native development or release build.

`ai.kratos.mobile` is the current development Android application ID. Confirm
the release identifier before publishing a signed build.

Build the Android Tailcat artifact with the pinned Go 1.27.1 and `gomobile`
toolchain:

```sh
source Native/env.sh
./Native/build-aar.sh
```
