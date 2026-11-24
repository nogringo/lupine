// A lupine raceme: whorls of pea florets up a stem, buds paling toward the tip.

const COLORS = ["#5b3fd6", "#6a4fe4", "#7c63ef", "#937ef5", "#ae9cf8", "#c9bdfb", "#dcd6f0"];

interface Floret {
  x: number;
  y: number;
  size: number;
  color: string;
  index: number;
}

function florets(whorls: number, height: number): Floret[] {
  const result: Floret[] = [];
  const step = (height * 0.78) / whorls;
  for (let w = 0; w < whorls; w++) {
    const t = w / (whorls - 1);
    const size = 17 - 11 * t;
    const y = height * 0.86 - w * step;
    const spread = size * 1.25;
    const offsets = w % 2 === 0 ? [-spread, 0, spread] : [-spread * 0.55, spread * 0.55];
    const color = COLORS[Math.min(COLORS.length - 1, Math.floor(t * COLORS.length))];
    for (const dx of offsets) {
      const jitter = Math.sin(w * 12.9898 + dx) * size * 0.12;
      result.push({ x: 100 + dx + jitter, y: y + jitter, size, color, index: result.length });
    }
  }
  return result;
}

export function Flower({ whorls = 24, className }: { whorls?: number; className?: string }) {
  const height = 640;
  const parts = florets(whorls, height);
  return (
    <svg className={className} viewBox={`0 0 200 ${height}`} aria-hidden="true">
      <path d={`M100 ${height} C 98 ${height * 0.6}, 103 ${height * 0.3}, 100 ${height * 0.06}`} stroke="#3f7a57" strokeWidth="4" fill="none" />
      {[-1, 1].map((side) => (
        <g key={side} fill="#3f7a57" opacity="0.9">
          {[0, 1, 2, 3, 4].map((leaf) => (
            <ellipse
              key={leaf}
              cx={100 + side * 30}
              cy={height - 50}
              rx="30"
              ry="6"
              transform={`rotate(${side * (leaf * 22 - 44)} 100 ${height - 50})`}
            />
          ))}
        </g>
      ))}
      {parts.map((f) => (
        <g key={f.index} className="floret" style={{ "--i": f.index } as React.CSSProperties}>
          <ellipse cx={f.x} cy={f.y - f.size * 0.25} rx={f.size * 0.78} ry={f.size * 0.62} fill={f.color} />
          <ellipse cx={f.x} cy={f.y + f.size * 0.3} rx={f.size * 0.42} ry={f.size * 0.32} fill="#f1edff" opacity="0.85" />
        </g>
      ))}
    </svg>
  );
}

/** The flower as a mark, for the top bar. */
export function Mark() {
  return (
    <svg className="mark" viewBox="0 0 32 32" aria-hidden="true">
      <path d="M16 31V5" stroke="#3f7a57" strokeWidth="2" />
      {[
        [25, 4, 3.2, "#5b3fd6"],
        [19, 3.4, 2.8, "#7c63ef"],
        [13.5, 2.8, 2.3, "#ae9cf8"],
      ].map(([y, rx, ry, color]) => (
        <g key={y} fill={color as string}>
          <ellipse cx={16 - (rx as number)} cy={y} rx={rx} ry={ry} />
          <ellipse cx={16 + (rx as number)} cy={y} rx={rx} ry={ry} />
        </g>
      ))}
      <ellipse cx="16" cy="8" rx="2.2" ry="2" fill="#c9bdfb" />
    </svg>
  );
}
