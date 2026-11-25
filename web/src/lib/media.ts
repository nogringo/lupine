const TYPES: Record<string, string> = {
  aac: "audio/aac",
  avif: "image/avif",
  flac: "audio/flac",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  json: "application/json",
  m4a: "audio/mp4",
  md: "text/markdown",
  mov: "video/quicktime",
  mp3: "audio/mpeg",
  mp4: "video/mp4",
  ogg: "audio/ogg",
  opus: "audio/ogg",
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

export type Viewable = "image" | "video" | "audio" | "pdf";

/** What the viewer shows a file as, if it shows it at all. */
export function viewable(name: string): Viewable | undefined {
  const type = mimeType(name);
  if (type.startsWith("image/")) return "image";
  if (type.startsWith("video/")) return "video";
  if (type.startsWith("audio/")) return "audio";
  if (type === "application/pdf" && navigator.pdfViewerEnabled) return "pdf";
  return undefined;
}
