export type Tone = "ok" | "warn" | "bad" | "idle";

const colors: Record<Tone, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
  idle: "bg-faint",
};

export function StatusDot({ tone }: { tone: Tone }) {
  return <span className={`inline-block size-2 shrink-0 rounded-full ${colors[tone]}`} />;
}
