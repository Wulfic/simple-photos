/**
 * SmartClusterList — shared list view for the auto-generated "cluster" smart
 * albums (Trips, Memories, Pets, People). Each of those used to hand-roll the
 * same scaffold: AppHeader, a back-to-Albums header, a loading skeleton, an
 * empty state, and a grid of representative-thumbnail cards. They differ only
 * in their data source, card layout (rectangular vs circular avatar), labels,
 * and per-card text — all expressed here as props.
 *
 * Representative thumbnails come from the shared useIdbThumbnailMap hook.
 */
import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useAppNavigate } from "../../hooks/useAppNavigate";
import AppHeader from "../../components/AppHeader";
import { GallerySkeleton } from "../../components/skeletons";
import DetailHeader from "../../components/DetailHeader";
import { useIdbThumbnailMap } from "../../hooks/useIdbThumbnailMap";

export interface ClusterCard {
  /** Stable id — used as the thumbnail-map key and the React key. */
  key: string | number;
  /** Representative server photo id (or null to show the placeholder). */
  photoId: string | null;
  /** Route navigated to when the card is clicked. */
  href: string;
  /** Card title (city / memory name / pet or person label). */
  title: string;
  /** `alt` text for the thumbnail image. */
  alt: string;
  /** Muted text line(s) rendered under the title. */
  meta: ReactNode;
  /** Optional CSS applied to the thumbnail <img> — used by People to zoom the
   *  tile onto the detected face (see computeFaceCropStyle). */
  imgStyle?: CSSProperties;
}

interface SmartClusterListProps<T> {
  title: string;
  /** Back-arrow destination. Defaults to the Albums index. */
  backTo?: string;
  emptyTitle: string;
  emptyHint?: string;
  /** "card" = rectangular landscape tile; "avatar" = circular portrait tile. */
  variant: "card" | "avatar";
  /** Placeholder icon rendered when a card has no thumbnail. */
  placeholder: ReactNode;
  /** Extra classes for the card title (e.g. "capitalize" for pets). */
  titleClassName?: string;
  /** Fetch the clusters. */
  load: () => Promise<T[]>;
  /** Map one cluster to its card descriptor. */
  toCard: (item: T) => ClusterCard;
  /** Server-thumbnail fallback used when no cached thumbnail exists. */
  fallbackUrl?: (photoId: string) => string;
  /** When set, replaces the default `navigate(card.href)` on card click —
   *  used by the "assign a face to this person" picker flow. */
  onCardClick?: (item: T) => void;
  /** Optional banner rendered above the grid (e.g. the assign-mode prompt). */
  notice?: ReactNode;
}

const GRID_CLASS: Record<"card" | "avatar", string> = {
  card: "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-4",
  avatar: "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4",
};

export default function SmartClusterList<T>({
  title,
  backTo = "/albums",
  emptyTitle,
  emptyHint,
  variant,
  placeholder,
  titleClassName,
  load,
  toCard,
  fallbackUrl,
  onCardClick,
  notice,
}: SmartClusterListProps<T>) {
  const navigate = useAppNavigate();
  const [items, setItems] = useState<T[]>([]);
  const [loading, setLoading] = useState(true);

  // Ref so an inline `load` closure doesn't need to be an effect dependency.
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await loadRef.current();
        if (!cancelled) setItems(data);
      } catch { /* feature (geo / AI) may not be enabled */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, []);

  const entries = items.map((item) => ({ item, card: toCard(item) }));
  const thumbUrls = useIdbThumbnailMap(
    entries.map(({ card }) => ({ key: card.key, photoId: card.photoId })),
    fallbackUrl ? { fallbackUrl } : undefined,
  );

  return (
    <div className="min-h-screen bg-canvas">
      <AppHeader />
      <main className="p-4">
        <DetailHeader backTo={backTo} backTitle="Back to Albums" title={title} />

        {notice}

        {loading ? (
          <GallerySkeleton />
        ) : entries.length === 0 ? (
          <div className="text-center py-12 border-2 border-dashed border-edge-strong rounded-lg">
            <p className="text-fg-muted">{emptyTitle}</p>
            {emptyHint && <p className="text-fg-muted text-sm mt-1">{emptyHint}</p>}
          </div>
        ) : (
          <div className={GRID_CLASS[variant]}>
            {entries.map(({ item, card }) => (
              <ClusterTile
                key={card.key}
                card={card}
                variant={variant}
                titleClassName={titleClassName}
                thumbUrl={thumbUrls[card.key]}
                placeholder={placeholder}
                onClick={() => (onCardClick ? onCardClick(item) : navigate(card.href))}
              />
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

function ClusterTile({
  card,
  variant,
  titleClassName,
  thumbUrl,
  placeholder,
  onClick,
}: {
  card: ClusterCard;
  variant: "card" | "avatar";
  titleClassName?: string;
  thumbUrl?: string;
  placeholder: ReactNode;
  onClick: () => void;
}) {
  if (variant === "avatar") {
    return (
      <div onClick={onClick} className="card card-interactive p-3 cursor-pointer">
        <div className="aspect-square bg-surface-raised rounded-full mb-2 mx-auto w-24 h-24 flex items-center justify-center overflow-hidden">
          {thumbUrl ? (
            <img src={thumbUrl} alt={card.alt} style={card.imgStyle} className="w-full h-full object-cover rounded-full" />
          ) : (
            placeholder
          )}
        </div>
        <p className={`font-medium text-center text-sm truncate ${titleClassName ?? ""}`}>{card.title}</p>
        {card.meta}
      </div>
    );
  }
  return (
    <div onClick={onClick} className="card card-interactive cursor-pointer overflow-hidden">
      <div className="aspect-video bg-surface-raised flex items-center justify-center overflow-hidden">
        {thumbUrl ? (
          <img src={thumbUrl} alt={card.alt} style={card.imgStyle} className="w-full h-full object-cover" />
        ) : (
          placeholder
        )}
      </div>
      <div className="p-3">
        <p className={`font-medium text-sm truncate ${titleClassName ?? ""}`}>{card.title}</p>
        {card.meta}
      </div>
    </div>
  );
}
