const TYPES: Record<string, string> = {
  avif: "image/avif",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  json: "application/json",
  md: "text/markdown",
  mov: "video/quicktime",
  mp3: "audio/mpeg",
  mp4: "video/mp4",
  pdf: "application/pdf",
  png: "image/png",
  svg: "image/svg+xml",
  txt: "text/plain",
  wav: "audio/wav",
  webm: "video/webm",
  webp: "image/webp",
};

export function mimeType(name: string): string {
  const dot = name.lastIndexOf(".");
  return (dot > 0 && TYPES[name.slice(dot + 1).toLowerCase()]) || "application/octet-stream";
}

/** What the viewer shows a file as, if it shows it at all. */
export function viewable(name: string): "image" | "video" | undefined {
  const type = mimeType(name);
  if (type.startsWith("image/")) return "image";
  if (type.startsWith("video/")) return "video";
  return undefined;
}
