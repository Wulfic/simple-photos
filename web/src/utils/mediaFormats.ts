/**
 * Maps non-browser-native file extensions to their converted format.
 * Mirrors the server-side `needs_web_preview()` function in scan.rs.
 *
 * Returns the converted extension (e.g. "mp4", "jpg") or null if the format
 * is already browser-native and no conversion is needed.
 */
export function getConvertedFormat(filename: string): string | null {
  const ext = filename.split(".").pop()?.toLowerCase();
  if (!ext) return null;

  switch (ext) {
    // Images that browsers cannot display natively
    case "heic":
    case "heif":
    case "tiff":
    case "tif":
    case "hdr":
    case "cr2":
    case "cur":
    case "cursor":
    case "dng":
    case "nef":
    case "arw":
    case "raw":
      return "jpg";

    // SVG / ICO → rasterized PNG
    case "svg":
    case "ico":
      return "png";

    // Audio that browsers cannot play natively
    case "wma":
    case "aiff":
    case "aif":
      return "mp3";

    // Video containers that browsers cannot play natively
    case "mkv":
    case "avi":
    case "wmv":
    case "asf":
        case "h264":
    case "h265":
    case "mpg":
    case "mpeg":
    case "3gp":
    case "mov":
    case "m4v":
      return "mp4";

    default:
      return null;
  }
}

/** Human-friendly label for a file extension */
export function formatLabel(ext: string): string {
  return ext.toUpperCase();
}
