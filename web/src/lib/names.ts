/** How the drive compares names: NFC, then case folded. */
export function folded(name: string): string {
  return name.normalize("NFC").toLowerCase();
}

/** The first of `name`, `stem (2).ext`, `stem (3).ext`... that `taken` lacks. */
export function freeName(name: string, taken: Set<string>): string {
  if (!taken.has(folded(name))) return name;
  const dot = name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  for (let n = 2; ; n++) {
    const candidate = `${stem} (${n})${ext}`;
    if (!taken.has(folded(candidate))) return candidate;
  }
}

/** Why the drive would refuse `name`, if it would. */
export function nameProblem(name: string): string | undefined {
  if (name.trim() === "") return "Enter a name.";
  if (name === "." || name === "..") return "This name is reserved.";
  if (name.includes("/")) return "Names cannot contain /.";
  if (new TextEncoder().encode(name.normalize("NFC")).length > 255) return "This name is too long.";
  return undefined;
}
