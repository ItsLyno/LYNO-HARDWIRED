import type { ReactNode } from "react";

// The `//` echoes the LYNO//HARDWIRED mark; it is the only accent on a page besides the primary button.
export function PageTitle({ children, sub }: { children: ReactNode; sub?: ReactNode }) {
  return (
    <div>
      <h1 className="text-2xl font-semibold tracking-tight">
        <span className="mr-2 font-mono font-normal text-accent">//</span>
        {children}
      </h1>
      {sub && <p className="mt-1.5 font-mono text-xs text-muted tabular-nums">{sub}</p>}
    </div>
  );
}
