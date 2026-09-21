const commandPattern = /^([0-9a-f]{40,64}) [0-9a-f]{40,64} (refs\/heads\/main)(?:\0|$)/i

export function parseReceiveCommand(body) {
  if (!(body instanceof Uint8Array) || body.length < 4) return undefined
  const packetLength = Number.parseInt(new TextDecoder().decode(body.subarray(0, 4)), 16)
  if (!Number.isSafeInteger(packetLength) || packetLength <= 4 || packetLength > body.length) return undefined
  const command = new TextDecoder().decode(body.subarray(4, packetLength))
  const match = commandPattern.exec(command)
  if (!match || /^0+$/.test(match[1])) return undefined
  return {oldOid: match[1].toLowerCase(), ref: match[2]}
}
