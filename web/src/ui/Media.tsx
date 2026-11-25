import type { Viewable } from "../lib/media";

export const UNSUPPORTED: Record<Viewable, string> = {
  image: "This browser cannot show this image.",
  video: "This browser cannot play this video.",
  audio: "This browser cannot play this audio file.",
  pdf: "This browser cannot show this PDF.",
};

/** A file the browser shows, from a blob URL. */
export function Media({ media, url, name, onError }: { media: Viewable; url: string; name: string; onError: () => void }) {
  switch (media) {
    case "image":
      return <img src={url} alt={name} onError={onError} />;
    case "video":
      return <video src={url} controls autoPlay onError={onError} />;
    case "audio":
      return <audio src={url} controls autoPlay onError={onError} />;
    case "pdf":
      // No sandbox: Chrome shows no PDF in one. The browser's PDF viewer keeps the file's scripts from this page.
      return <iframe src={url} title={name} />;
  }
}
