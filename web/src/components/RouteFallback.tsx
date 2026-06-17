/**
 * RouteFallback — Suspense fallback for lazily code-split protected routes.
 *
 * Renders the shared AppHeader (so it stays in place while the route's JS chunk
 * downloads) above a generic photo-grid skeleton in the standard page
 * container. Most protected routes are grid pages, so this reads as an
 * intentional loading state rather than a blank flash. The skeleton's shimmer /
 * reduced-motion handling come from the underlying primitive.
 */
import AppHeader from "./AppHeader";
import { GallerySkeleton } from "./skeletons";

export default function RouteFallback() {
  return (
    <>
      <AppHeader />
      <main className="p-4">
        <GallerySkeleton />
      </main>
    </>
  );
}
