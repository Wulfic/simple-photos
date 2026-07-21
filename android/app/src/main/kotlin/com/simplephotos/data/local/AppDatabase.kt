/**
 * Room database definition and DAO accessors for local photo metadata,
 * albums, blob upload queue, and backup folder selections.
 */
package com.simplephotos.data.local

import androidx.room.Database
import androidx.room.RoomDatabase
import androidx.room.TypeConverters
import com.simplephotos.data.local.dao.AlbumDao
import com.simplephotos.data.local.dao.BackupFolderDao
import com.simplephotos.data.local.dao.BlobQueueDao
import com.simplephotos.data.local.dao.PhotoDao
import com.simplephotos.data.local.entities.*

/**
 * Room database for the Simple Photos Android client.
 *
 * Stores local photo metadata, album manifests, the blob upload queue, and
 * backup folder selections. Uses [fallbackToDestructiveMigration] so schema
 * changes during development wipe the cache rather than crashing.
 */
@Database(
    entities = [PhotoEntity::class, AlbumEntity::class, PhotoAlbumXRef::class, BlobQueueEntity::class, BackupFolderEntity::class],
    // 12: PhotoEntity.renditions — the #49 video resolution ladder. The
    // destructive migration drops the mirror, which the next sync rebuilds from
    // a full walk; that is also what re-populates the ladder for every video.
    version = 12,
    exportSchema = false
)
@TypeConverters(Converters::class)
abstract class AppDatabase : RoomDatabase() {
    abstract fun photoDao(): PhotoDao
    abstract fun albumDao(): AlbumDao
    abstract fun blobQueueDao(): BlobQueueDao
    abstract fun backupFolderDao(): BackupFolderDao
}
