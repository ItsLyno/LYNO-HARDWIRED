import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { api } from "./api";
import defaultImage from "./assets/background.gif";

export interface Background {
  source: "default" | "custom" | "none";
  /** The player's file for `custom`; read through the asset protocol, never copied. */
  path: string | null;
  /** Darkening in the middle of the window, percent; the edges get 60% of it. */
  dim: number;
  /** Pixels. */
  blur: number;
  /** Hard pixel edges instead of smoothing: the default picture is pixel art. */
  pixelated: boolean;
}

// A UI preference of this machine, like the collapsed sections of the mod list: localStorage, not Settings.
export const useBackground = create<Background>()(
  persist(() => ({ source: "default", path: null, dim: 80, blur: 0, pixelated: true }) as Background, {
    name: "lyno-background",
    // v1: the veil got stronger, so the old default of the dim would show through; reset it once.
    version: 1,
    migrate: (old) => ({ ...(old as Background), dim: 80 }),
  }),
);

export function BackgroundLayer() {
  const { source, path, dim, blur, pixelated } = useBackground();
  // The asset protocol's scope is empty at start: the picked file is opened to it on every launch.
  const [allowed, setAllowed] = useState<string | null>(null);
  useEffect(() => {
    if (source === "custom" && path) api.allowBackground(path).then(() => setAllowed(path), () => setAllowed(null));
  }, [source, path]);
  const src =
    source === "default" ? defaultImage : source === "custom" && path && allowed === path ? convertFileSrc(path) : null;
  if (!src) return null;
  return (
    <div aria-hidden className="pointer-events-none fixed inset-0 -z-10 overflow-hidden">
      <img
        src={src}
        alt=""
        draggable={false}
        // Blur pulls transparent edges in; the scale pushes them outside the window.
        style={{ filter: blur ? `blur(${blur}px)` : undefined, imageRendering: pixelated ? "pixelated" : "auto" }}
        className={`size-full object-cover ${blur ? "scale-110" : ""}`}
      />
      {/* Darkest behind the content in the middle, fading towards the edges where the picture shows. */}
      <div
        className="absolute inset-0"
        style={{ background: `radial-gradient(ellipse at center, rgb(0 0 0 / ${dim / 100}), rgb(0 0 0 / ${dim / 167}) 100%)` }}
      />
    </div>
  );
}
