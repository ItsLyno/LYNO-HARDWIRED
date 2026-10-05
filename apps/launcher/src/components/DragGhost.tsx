import { ArrowDownUp, Package } from "lucide-react";
import { useEffect, useRef } from "react";
import { api } from "../api";
import { dropSpotAt, useApp } from "../store";

/** Pixels the pointer moves before a press becomes a drag. */
const DRAG_THRESHOLD = 5;

/** Starts `drag` once a press moves far enough. Pointer events, not HTML5 drag and drop: WebView2 drops those while
 *  Tauri listens for Explorer files. The click that ends a drag doesn't reach the row (it would open its menu). */
export function pressToDrag(e: React.PointerEvent, drag: (ev: PointerEvent) => void) {
  if (e.button !== 0) return;
  const start = { x: e.clientX, y: e.clientY };
  const move = (ev: PointerEvent) => {
    if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < DRAG_THRESHOLD) return;
    cleanup();
    // Neither does the press select the list's text on its way.
    const swallow = (c: MouseEvent) => c.stopPropagation();
    window.addEventListener("click", swallow, true);
    document.body.classList.add("select-none");
    window.getSelection()?.removeAllRanges();
    window.addEventListener(
      "pointerup",
      () => {
        document.body.classList.remove("select-none");
        setTimeout(() => window.removeEventListener("click", swallow, true));
      },
      { once: true },
    );
    drag(ev);
  };
  const cleanup = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", cleanup);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", cleanup);
}

/** What is dragged, under the pointer: an archive from the downloads list, or a row of the player's own mods. */
export function DragGhost() {
  const drag = useApp((s) => s.drag);
  const active = drag !== null;
  const ghost = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!active) return;
    const d0 = useApp.getState().drag!;
    // The ghost follows the pointer by itself; the store, and with it the mod list, changes only with the drop spot.
    const move = (e: PointerEvent) => {
      ghost.current?.style.setProperty("translate", `${e.clientX - d0.x}px ${e.clientY - d0.y}px`);
      const d = useApp.getState().drag;
      const over = dropSpotAt(e.clientX, e.clientY);
      if (d && (over?.after !== d.over?.after || over?.build !== d.over?.build || !over !== !d.over)) useApp.setState({ drag: { ...d, over } });
    };
    const up = (e: PointerEvent) => {
      const d = useApp.getState().drag;
      useApp.setState({ drag: null });
      const spot = dropSpotAt(e.clientX, e.clientY);
      // A separator of the player stays in their section; their mods go among the build's too.
      if (!d || !spot || (d.move && movesSeparator(d) && spot.build)) return;
      const { run, refreshUserMods, installArchive } = useApp.getState();
      const files = d.files ?? [d.file];
      if (!d.move) void installArchive(d.file, spot.after);
      else if (!(spot.after && files.includes(spot.after)))
        // Each one under the one before it: the selection lands as a block, in its order.
        void run(async () => {
          let after = spot.after;
          for (const f of files) {
            await api.moveUserMod(f, after);
            after = f;
          }
          await refreshUserMods();
        });
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
  const Icon = drag.move ? ArrowDownUp : Package;
  const blocked = drag.move && movesSeparator(drag) && drag.over?.build;
  return (
    <div
      ref={ghost}
      className="pointer-events-none fixed z-[60] flex max-w-96 items-center gap-2 rounded-full bg-raised px-3 py-1.5 text-[13px] shadow-xl ring-1 ring-neon/40"
      style={{ left: drag.x + 14, top: drag.y + 10 }}
    >
      <Icon size={13} className="shrink-0 text-neon" />
      <span className="max-w-40 shrink-0 truncate">{drag.label}</span>
      <span className={`truncate ${blocked ? "text-warn" : "text-faint"}`}>
        {blocked
          ? "· только среди ваших модов"
          : !drag.over
            ? drag.move
              ? ""
              : "· в список модов"
            : drag.over.after && !drag.over.after.endsWith("_separator")
              ? `· после ${drag.over.after}`
              : "· сюда"}
      </span>
    </div>
  );
}

/** A separator of the player moves only within their section: MO2 would put build mods under it. */
export function movesSeparator(d: { file: string; files?: string[] }): boolean {
  return (d.files ?? [d.file]).some((f) => f.endsWith("_separator"));
}
