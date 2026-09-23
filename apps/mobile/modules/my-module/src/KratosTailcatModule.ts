import { NativeModule, requireOptionalNativeModule } from 'expo';

declare class KratosTailcatModule extends NativeModule<{}> {
  connect(invitation: string): Promise<string>;
  restore(): Promise<string | null>;
  renew(): Promise<string>;
  disconnect(): void;
}

export default requireOptionalNativeModule<KratosTailcatModule>('KratosTailcat');
