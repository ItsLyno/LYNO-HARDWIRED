export type Tone = "ok" | "warn" | "bad" | "idle";

const colors: Record<Tone, string> = {
  ok: "bg-ok shadow-[0_0_8px_var(--color-ok)]",
  warn: "bg-warn shadow-[0_0_8px_var(--color-warn)]",
  bad: "bg-bad shadow-[0_0_8px_var(--color-bad)]",
  idle: "bg-faint",
};

export function StatusDot({ tone }: { tone: Tone }) {
  return <span className={`inline-block size-1.5 shrink-0 rounded-full ${colors[tone]}`} />;
}
