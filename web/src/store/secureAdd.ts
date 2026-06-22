/**
 * Secure-add session — tracks an in-progress "add photos to a secure album"
 * flow that spans pages.
 *
 * When the user taps "Add Photos" inside a secure album we send them to the
 * regular Albums page to browse and pick photos (rather than scrolling a giant
 * flat master list). This store holds the target secure album for the duration
 * of that flow so every photo grid (smart albums + regular albums) can offer an
 * "Add to 🔒 <name>" action, and the Albums page can show a banner.
 *
 * Kept in-memory (Zustand). The whole flow is in-app SPA navigation, so it
 * survives route changes; a full page reload cancels it (the user just restarts
 * from the secure album).
 */
import { create } from "zustand";

interface SecureAddTarget {
  galleryId: string;
  galleryName: string;
}

interface SecureAddState {
  target: SecureAddTarget | null;
  start: (galleryId: string, galleryName: string) => void;
  cancel: () => void;
}

export const useSecureAdd = create<SecureAddState>((set) => ({
  target: null,
  start: (galleryId, galleryName) => set({ target: { galleryId, galleryName } }),
  cancel: () => set({ target: null }),
}));
