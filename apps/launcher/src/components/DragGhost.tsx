import { Package } from "lucide-react";
import { useEffect } from "react";
import { dropSpotAt, useApp } from "../store";

/** The archive under the pointer while it is dragged from the downloads list; drops it on release. */
export function DragGhost() {
  const drag = useApp((s) => s.drag);
  const active = drag !== null;

  useEffect(() => {
    if (!active) return;
    const move = (e: PointerEvent) => {
      const d = useApp.getState().drag;
      if (d) useApp.setState({ drag: { ...d, x: e.clientX, y: e.clientY, over: dropSpotAt(e.clientX, e.clientY) } });
    };
    const up = (e: PointerEvent) => {
      const d = useApp.getState().drag;
      useApp.setState({ drag: null });
      const spot = dropSpotAt(e.clientX, e.clientY);
      if (d && spot) void useApp.getState().installArchive(d.file, spot.after);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && useApp.setState({ drag: null });
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", esc);
    };
  }, [active]);

  if (!drag) return null;
  return (
    <div
      className="pointer-events-none fixed z-[60] flex max-w-72 items-center gap-2 rounded-full bg-raised px-3 py-1.5 text-[13px] shadow-xl ring-1 ring-neon/40"
      style={{ left: drag.x + 14, top: drag.y + 10 }}
    >
      <Package size={13} className="shrink-0 text-neon" />
      <span className="truncate">{drag.label}</span>
      {!drag.over && <span className="shrink-0 text-faint">· в список модов</span>}
    </div>
  );
}
