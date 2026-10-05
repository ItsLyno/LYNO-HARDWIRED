export function Chip(props: { active: boolean; onClick: () => void; count?: number; children: string }) {
  return (
    <button
      onClick={props.onClick}
      className={`h-8 rounded-full px-3.5 text-[13px] transition-colors ${
        props.active ? "bg-raised text-fg" : "text-muted hover:bg-raised/60 hover:text-fg"
      }`}
    >
      {props.children}
      {props.count !== undefined && <span className="ml-1.5 font-mono text-[11px] text-faint tabular-nums">{props.count}</span>}
    </button>
  );
}
