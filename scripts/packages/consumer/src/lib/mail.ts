// An app's own data types on top of @roedy/jmap (its README › Extending it in an app).
import { registerCapabilities, type GetArgs, type GetResponse } from '@roedy/jmap';

export const MAIL = 'urn:ietf:params:jmap:mail';

export interface Mailbox {
  id: string;
  name: string;
}

declare module '@roedy/jmap' {
  interface Methods {
    'Mailbox/get': { args: GetArgs<Mailbox>; response: GetResponse<Mailbox> };
  }
}

registerCapabilities({ Mailbox: MAIL });
