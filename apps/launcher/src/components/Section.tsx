import type { ReactNode } from "react";

export function Section(props: { title: string; action?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`panel ${props.className ?? ""}`}>
      <div className="flex h-12 items-center justify-between px-5">
        <h2 className="label">{props.title}</h2>
        {props.action}
      </div>
      {props.children}
    </section>
  );
}
