import re

file_path = 'web/src/components/gallery/MediaTile.tsx'
with open(file_path, 'r') as f:
    content = f.read()

content = content.replace(
    'import { thumbnailSrc, formatDuration } from "../../utils/gallery";',
    'import { thumbnailSrc, formatDuration } from "../../utils/gallery";\nimport { useAuthStore } from "../../store/auth";'
)

old_effect = '''  useEffect(() => {
    if (visible && photo.thumbnailData) {
      const url = thumbnailSrc(photo.thumbnailData, thumbMime(photo));
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    }
  }, [visible, photo.thumbnailData, photo.thumbnailMimeType, photo.mediaType]);'''

new_effect = '''  useEffect(() => {
    if (!visible) return;
    
    if (photo.thumbnailData) {
      const url = thumbnailSrc(photo.thumbnailData, thumbMime(photo));
      setSrc(url);
      return () => URL.revokeObjectURL(url);
    } else if (photo.serverSide && photo.serverPhotoId) {
      const token = useAuthStore.getState().accessToken;
      setSrc(`/api/photos/${photo.serverPhotoId}/thumbnail?token=${token}`);
    }
  }, [visible, photo.thumbnailData, photo.serverSide, photo.serverPhotoId, photo.thumbnailMimeType, photo.mediaType]);'''

content = content.replace(old_effect, new_effect)

with open(file_path, 'w') as f:
    f.write(content)
