import { registerWebModule, NativeModule } from 'expo';

class KratosTailcatModule extends NativeModule<{}> {
  async connect(): Promise<string> {
    throw new Error('Tailcat requires an Android development build.');
  }

  async restore(): Promise<string | null> { return null; }
  async renew(): Promise<string> { throw new Error('Tailcat requires an Android development build.'); }
  disconnect(): void {}
}

export default registerWebModule(KratosTailcatModule, 'KratosTailcatModule');
