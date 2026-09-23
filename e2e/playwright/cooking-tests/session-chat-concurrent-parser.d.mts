export type ParsedReceiveCommand = {
  oldOid: string;
  ref: string;
};

export function parseReceiveCommand(body: Uint8Array): ParsedReceiveCommand | undefined;
