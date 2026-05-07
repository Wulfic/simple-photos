/**
 * Minimal ambient type declarations for the W3C Presentation API.
 * (TypeScript's bundled lib.dom.d.ts omits these in ES2020 target builds.)
 * https://www.w3.org/TR/presentation-api/
 */

type PresentationConnectionState = "connecting" | "connected" | "closed" | "terminated";

interface PresentationConnection extends EventTarget {
  readonly id: string;
  readonly url: string;
  readonly state: PresentationConnectionState;
  onconnect: ((this: PresentationConnection, ev: Event) => unknown) | null;
  onclose: ((this: PresentationConnection, ev: Event) => unknown) | null;
  onterminate: ((this: PresentationConnection, ev: Event) => unknown) | null;
  onmessage: ((this: PresentationConnection, ev: MessageEvent) => unknown) | null;
  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void;
  close(): void;
  terminate(): void;
}

interface PresentationAvailability extends EventTarget {
  readonly value: boolean;
  onchange: ((this: PresentationAvailability, ev: Event) => unknown) | null;
}

interface PresentationConnectionList extends EventTarget {
  readonly connections: ReadonlyArray<PresentationConnection>;
  onconnectionavailable: ((
    this: PresentationConnectionList,
    ev: PresentationConnectionAvailableEvent
  ) => unknown) | null;
}

interface PresentationConnectionAvailableEvent extends Event {
  readonly connection: PresentationConnection;
}

interface PresentationReceiver {
  readonly connectionList: Promise<PresentationConnectionList>;
}

interface PresentationRequest extends EventTarget {
  start(): Promise<PresentationConnection>;
  reconnect(presentationId: string): Promise<PresentationConnection>;
  getAvailability(): Promise<PresentationAvailability>;
}

declare class PresentationRequest implements PresentationRequest {
  constructor(urls: string | string[]);
}

interface Presentation {
  defaultRequest: PresentationRequest | null;
  readonly receiver: PresentationReceiver | null;
}

interface Navigator {
  readonly presentation: Presentation | undefined;
}

interface Window {
  PresentationRequest: typeof PresentationRequest | undefined;
}
