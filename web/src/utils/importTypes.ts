/** Encryption mode for photo storage */
export type EncryptionMode = "plain" | "encrypted";

/** Google Photos JSON metadata shape */
export interface GooglePhotosMetadata {
  title: string;
  description?: string;
  imageViews?: string;
  creationTime?: { timestamp: string; formatted: string };
  photoTakenTime?: { timestamp: string; formatted: string };
  geoData?: {
    latitude: number;
    longitude: number;
    altitude: number;
    latitudeSpan?: number;
    longitudeSpan?: number;
  };
  geoDataExif?: {
    latitude: number;
    longitude: number;
    altitude: number;
  };
  url?: string;
  googlePhotosOrigin?: Record<string, unknown>;
}

/** A single item to be imported */
export interface ImportItem {
  /** For local files */
  file?: File;
  /** For server files — the absolute path on server */
  serverPath?: string;
  /** File name */
  name: string;
  /** File size in bytes */
  size: number;
  /** MIME type */
  mimeType: string;
  /** Google Photos metadata (optional) */
  metadata?: GooglePhotosMetadata;
  metadataFile?: string;
  status: "pending" | "uploading" | "done" | "error";
  error?: string;
}

/** Server-provided file listing entry */
export interface ServerFile {
  name: string;
  path: string;
  size: number;
  mime_type: string;
  modified: string | null;
}
