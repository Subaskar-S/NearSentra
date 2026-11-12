const YOCTO = 10n ** 24n;

/** Format a yoctoNEAR decimal string as NEAR with up to `decimals` fractional digits. */
export function formatNear(yocto: string, decimals = 4): string {
  let v: bigint;
  try {
    v = BigInt(yocto);
  } catch {
    return '?';
  }
  const whole = v / YOCTO;
  const frac = (v % YOCTO).toString().padStart(24, '0').slice(0, decimals).replace(/0+$/, '');
  return frac ? `${whole}.${frac}` : whole.toString();
}

export function formatUtc(ms: number): string {
  return new Date(ms).toISOString().replace('T', ' ').replace(/\.\d+Z$/, ' UTC');
}

export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${m}m ${sec}s`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

/** Seconds until the epoch resets; <= 0 means it resets on the agent's next request. */
export function secondsUntilEpochReset(epochStartNs: string, epochDurationNs: string, nowMs: number): number {
  const resetAtMs = Number((BigInt(epochStartNs) + BigInt(epochDurationNs)) / 1_000_000n);
  return (resetAtMs - nowMs) / 1000;
}

export function shortHash(h: string): string {
  return h.length > 14 ? `${h.slice(0, 7)}…${h.slice(-5)}` : h;
}
