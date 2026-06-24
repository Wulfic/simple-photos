/**
 * Memories smart album — auto-generated location + date albums. The list and
 * per-memory detail views are thin configs over the shared SmartClusterList /
 * SmartAlbumDetail modules.
 */
import { api } from "../../api/client";
import SmartClusterList from "./SmartClusterList";
import SmartAlbumDetail from "./SmartAlbumDetail";
import { resolveServerPhotos } from "./resolveServerPhotos";

const MemoryIcon = (
  <svg className="w-8 h-8 text-fg-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.5}>
    <path strokeLinecap="round" strokeLinejoin="round" d="M15 10.5a3 3 0 11-6 0 3 3 0 016 0z" />
    <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 10.5c0 7.142-7.5 11.25-7.5 11.25S4.5 17.642 4.5 10.5a7.5 7.5 0 1115 0z" />
  </svg>
);

export function MemoriesView() {
  return (
    <SmartClusterList
      title="Memories"
      emptyTitle="No memories yet"
      emptyHint="Memories are auto-generated when you have 3+ photos from the same location and day"
      variant="card"
      placeholder={MemoryIcon}
      load={() => api.geo.listMemories()}
      toCard={(memory) => ({
        key: memory.id,
        photoId: memory.first_photo_id,
        href: `/albums/smart-memories/${memory.id}`,
        title: memory.name,
        alt: memory.name,
        meta: (
          <p className="text-xs text-fg-muted">
            {memory.photo_count} photo{memory.photo_count !== 1 ? "s" : ""} · {memory.country}
          </p>
        ),
      })}
    />
  );
}

export function MemoryDetailView({ memoryId }: { memoryId: string }) {
  return (
    <SmartAlbumDetail
      reloadKey={memoryId}
      defaultTitle="Memory"
      backTo="/albums/smart-memories"
      backLabel="Memories"
      viewerAlbumId={`smart-memories/${memoryId}`}
      emptyMessage="No photos found for this memory"
      load={async ({ setTitle }) => {
        const memories = await api.geo.listMemories();
        const memory = memories.find((m) => m.id === memoryId);
        if (memory) setTitle(memory.name);
        const summaries = await api.geo.listMemoryPhotos(memoryId);
        return resolveServerPhotos(summaries);
      }}
    />
  );
}
