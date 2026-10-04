import type { ReactNode } from "react";
import slashes from "../assets/slashes.webp";

// The slashes are cut out of the LYNO//HARDWIRED logo (assets/logo.webp), so every page heading carries the mark.
export function PageTitle({ children, sub }: { children: ReactNode; sub?: ReactNode }) {
  return (
    <div>
      <h1 className="flex items-center gap-3 text-2xl font-semibold tracking-tight">
        <img src={slashes} alt="" draggable={false} className="h-8 w-auto shrink-0" />
        {children}
      </h1>
      {sub && <p className="mt-1.5 font-mono text-xs text-muted tabular-nums">{sub}</p>}
    </div>
  );
}
