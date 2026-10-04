import type { ReactNode } from "react";

export function Section(props: { title: string; action?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`rounded-lg border border-line bg-surface ${props.className ?? ""}`}>
      <div className="flex h-12 items-center justify-between border-b border-line px-5">
        <h2 className="text-sm font-semibold">{props.title}</h2>
        {props.action}
      </div>
      {props.children}
    </section>
  );
}
